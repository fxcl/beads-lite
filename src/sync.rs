use crate::git::{self, GitError, PushOutcome};
use crate::jsonl::{self, IssueExport};
use crate::storage::Store;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSONL error: {0}")]
    Jsonl(#[from] crate::jsonl::JsonlError),
    #[error("Store error: {0}")]
    Store(#[from] crate::storage::StoreError),
    #[error("Git error: {0}")]
    Git(#[from] GitError),
}

pub type Result<T> = std::result::Result<T, SyncError>;

/// Exit-code mapping mirrors upstream `bd sync`:
/// 0 = ok, 2 = conflict (auto-merged via LWW but with diverged modifies),
/// 3 = retries exhausted (push race), 4 = dirty-stuck (reserved for future).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncStatus {
    Ok,
    Conflict,
    RetriesExhausted,
    NoRemote,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncOutcome {
    pub status: SyncStatus,
    pub attempts: u32,
    pub pulled: bool,
    pub pushed: bool,
    pub push_skipped: bool,
    pub merged_issues: usize,
    pub conflict_count: usize,
    pub last_error: Option<String>,
}

pub struct SyncEngine {
    store: Store,
    repo_path: PathBuf,
    /// Maximum push-pull attempts before returning RetriesExhausted.
    pub attempts: u32,
    /// If true, run pull + merge + commit but skip `git push`.
    pub no_push: bool,
}

impl SyncEngine {
    pub fn new(store: Store, repo_path: PathBuf) -> Self {
        Self {
            store,
            repo_path,
            attempts: 3,
            no_push: false,
        }
    }

    pub fn with_options(mut self, attempts: u32, no_push: bool) -> Self {
        self.attempts = attempts.max(1);
        self.no_push = no_push;
        self
    }

    pub fn run(&mut self) -> Result<SyncOutcome> {
        let mut outcome = SyncOutcome {
            status: SyncStatus::Ok,
            attempts: 0,
            pulled: false,
            pushed: false,
            push_skipped: self.no_push,
            merged_issues: 0,
            conflict_count: 0,
            last_error: None,
        };

        // 1. Pull
        if let Err(e) = git::run(&self.repo_path, &["pull", "--rebase"]) {
            // Upstream treats pull failure as transient; record and continue.
            // Local-only repos will hit this — that's expected, not an error.
            outcome.last_error = Some(format!("pull: {}", e));
        } else {
            outcome.pulled = true;
        }

        // 2. Load states
        let base = self.load_base()?;
        let local = self.load_local()?;
        let remote = self.load_remote()?;

        // 3. Merge (collect conflict count, then merge)
        let (merged, conflict_count) = self.merge(base, local, remote);
        outcome.merged_issues = merged.len();
        outcome.conflict_count = conflict_count;

        if conflict_count > 0 {
            outcome.status = SyncStatus::Conflict;
        }

        // 4. Apply changes
        self.apply_changes(merged)?;

        // 5. Commit
        git::run(&self.repo_path, &["add", "--", "issues.jsonl", "sync_base.jsonl"])?;
        if git::has_staged_changes(&self.repo_path)? {
            git::commit_snapshot(&self.repo_path, &["issues.jsonl", "sync_base.jsonl"], "bl sync")?;
        }

        // 6. Push (with retry)
        if self.no_push {
            // skipped — already recorded push_skipped=true
        } else {
            match git::push_with_retry(&self.repo_path, self.attempts) {
                Ok(PushOutcome::Pushed) => outcome.pushed = true,
                Ok(PushOutcome::Skipped) => outcome.push_skipped = true,
                Err(GitError::PushRace { attempts, .. }) => {
                    outcome.attempts = attempts;
                    outcome.status = SyncStatus::RetriesExhausted;
                }
                Err(e) => return Err(e.into()),
            }
        }

        Ok(outcome)
    }

    fn load_base(&self) -> Result<HashMap<String, IssueExport>> {
        let path = self.repo_path.join("sync_base.jsonl");
        if !path.exists() {
            return Ok(HashMap::new());
        }
        self.load_issues_from_file(&path)
    }

    fn load_remote(&self) -> Result<HashMap<String, IssueExport>> {
        let path = self.repo_path.join("issues.jsonl");
        if !path.exists() {
            return Ok(HashMap::new());
        }
        self.load_issues_from_file(&path)
    }

    fn load_local(&self) -> Result<HashMap<String, IssueExport>> {
        let issues = self.store.list_issues()?;
        let all_deps = self.store.get_all_dependencies()?;
        let mut all_comments: HashMap<String, Vec<crate::comment::Comment>> = HashMap::new();
        for c in self.store.list_all_comments()? {
            all_comments.entry(c.issue_id.clone()).or_default().push(c);
        }
        let mut map = HashMap::new();

        for issue in issues {
            let deps = all_deps.get(&issue.id).map(|v| v.as_slice()).unwrap_or(&[]);
            let comments = all_comments.get(&issue.id).map(|v| v.as_slice()).unwrap_or(&[]);
            let export = jsonl::to_issue_export(&issue, deps, comments);
            map.insert(export.id.clone(), export);
        }
        Ok(map)
    }

    fn load_issues_from_file(&self, path: &Path) -> Result<HashMap<String, IssueExport>> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut map = HashMap::new();

        for line in std::io::BufRead::lines(reader) {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let export: IssueExport = serde_json::from_str(&line).map_err(crate::jsonl::JsonlError::from)?;
            map.insert(export.id.clone(), export);
        }
        Ok(map)
    }

    /// 3-way merge logic. Returns merged state plus count of diverged-modify
    /// conflicts (auto-resolved via LWW per upstream semantics).
    pub fn merge(
        &self,
        base: HashMap<String, IssueExport>,
        local: HashMap<String, IssueExport>,
        remote: HashMap<String, IssueExport>,
    ) -> (Vec<IssueExport>, usize) {
        let mut merged = Vec::new();
        let mut conflicts = 0;
        let mut all_ids: HashSet<String> = HashSet::new();
        all_ids.extend(base.keys().cloned());
        all_ids.extend(local.keys().cloned());
        all_ids.extend(remote.keys().cloned());

        for id in all_ids {
            let b = base.get(&id);
            let l = local.get(&id);
            let r = remote.get(&id);

            match (b, l, r) {
                (None, Some(l_issue), None) => merged.push(l_issue.clone()),
                (None, None, Some(r_issue)) => merged.push(r_issue.clone()),
                (None, Some(l_issue), Some(r_issue)) => {
                    if l_issue.updated_at >= r_issue.updated_at {
                        merged.push(l_issue.clone());
                    } else {
                        merged.push(r_issue.clone());
                    }
                }
                (Some(_), None, None) => {}
                (Some(b_issue), None, Some(r_issue)) => {
                    if r_issue.updated_at > b_issue.updated_at {
                        merged.push(r_issue.clone());
                    }
                }
                (Some(b_issue), Some(l_issue), None) => {
                    if l_issue.updated_at > b_issue.updated_at {
                        merged.push(l_issue.clone());
                    }
                }
                (Some(b_issue), Some(l_issue), Some(r_issue)) => {
                    if l_issue.updated_at == r_issue.updated_at
                        || (l_issue.updated_at > b_issue.updated_at && r_issue.updated_at == b_issue.updated_at)
                    {
                        merged.push(l_issue.clone());
                    } else if r_issue.updated_at > b_issue.updated_at && l_issue.updated_at == b_issue.updated_at {
                        merged.push(r_issue.clone());
                    } else {
                        // Both changed divergently — count as conflict,
                        // auto-resolve via LWW.
                        conflicts += 1;
                        if l_issue.updated_at >= r_issue.updated_at {
                            merged.push(l_issue.clone());
                        } else {
                            merged.push(r_issue.clone());
                        }
                    }
                }
                (None, None, None) => unreachable!(),
            }
        }

        (merged, conflicts)
    }

    pub fn apply_changes(&mut self, merged: Vec<IssueExport>) -> Result<()> {
        let mut merged_jsonl = String::new();
        for item in &merged {
            let line = serde_json::to_string(item).map_err(crate::jsonl::JsonlError::from)?;
            merged_jsonl.push_str(&line);
            merged_jsonl.push('\n');
        }

        let current_issues = self.store.list_issues()?;
        let current_ids: HashSet<String> = current_issues.iter().map(|i| i.id.clone()).collect();
        let merged_ids: HashSet<String> = merged.iter().map(|i| i.id.clone()).collect();

        for id in current_ids {
            if !merged_ids.contains(&id) {
                self.store.delete_issue(&id)?;
            }
        }

        if !merged.is_empty() {
            crate::jsonl::import_from_jsonl(&mut self.store, std::io::Cursor::new(merged_jsonl.as_bytes()))?;
        }

        let mut sorted_merged = merged;
        sorted_merged.sort_by(|a, b| a.id.cmp(&b.id));

        let issues_path = self.repo_path.join("issues.jsonl");
        let file = File::create(&issues_path)?;
        let mut writer = BufWriter::new(file);
        for item in &sorted_merged {
            serde_json::to_writer(&mut writer, item).map_err(crate::jsonl::JsonlError::from)?;
            use std::io::Write;
            writeln!(writer)?;
        }

        let base_path = self.repo_path.join("sync_base.jsonl");
        let file = File::create(&base_path)?;
        let mut writer = BufWriter::new(file);
        for item in &sorted_merged {
            serde_json::to_writer(&mut writer, item).map_err(crate::jsonl::JsonlError::from)?;
            use std::io::Write;
            writeln!(writer)?;
        }

        Ok(())
    }
}
