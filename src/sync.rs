use crate::jsonl::{self, IssueExport};
use crate::storage::Store;
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
    Git(String),
}

pub type Result<T> = std::result::Result<T, SyncError>;

pub struct SyncEngine {
    store: Store,
    repo_path: PathBuf,
}

impl SyncEngine {
    pub fn new(store: Store, repo_path: PathBuf) -> Self {
        Self { store, repo_path }
    }

    fn git_exec(&self, args: &[&str]) -> Result<()> {
        let status = std::process::Command::new("git")
            .current_dir(&self.repo_path)
            .args(args)
            .status()
            .map_err(|e| SyncError::Io(e))?;

        if status.success() {
            Ok(())
        } else {
            // Check if failure is due to 'nothing to commit' or similar non-error states if needed
            // But for pull/push failure, we should error.
            Err(SyncError::Git(format!("git {:?} failed", args)))
        }
    }

    pub fn run(&mut self) -> Result<()> {
        println!("Syncing...");

        // 1. Git Pull (Rebase)
        println!("Pulling remote changes...");
        // This might fail if no remote or unrelated histories, but let's try.
        // We use --rebase to fetch and replay local commits on top of upstream.
        // But for our jsonl files, we want to just fetch and merge manually.
        // So maybe just 'git fetch' then 'git checkout origin/main -- issues.jsonl'?
        // No, 'bl sync' assumes full repo sync.
        
        // Let's use 'git pull --rebase' for simplicity now.
        if let Err(e) = self.git_exec(&["pull", "--rebase"]) {
             println!("Git pull failed (maybe no remote?): {}", e);
             // Verify if we can proceed. If local repo only, maybe ok?
             // But 'load_remote' depends on issues.jsonl being updated.
        }

        // 2. Load States
        let base = self.load_base()?;
        let local = self.load_local()?;
        let remote = self.load_remote()?;

        println!("Merging {} local, {} remote, {} base issues...", local.len(), remote.len(), base.len());

        // 3. Merge
        let merged = self.merge(base, local, remote);

        // 4. Apply Changes
        self.apply_changes(merged)?;

        // 5. Git Commit & Push
        println!("Committing and pushing changes...");
        self.git_exec(&["add", "issues.jsonl", "sync_base.jsonl"])?;

        // Check for changes
        let status = std::process::Command::new("git")
            .current_dir(&self.repo_path)
            .args(&["diff", "--cached", "--quiet"])
            .status()
            .map_err(|e| SyncError::Io(e))?;

        if !status.success() {
            self.git_exec(&["commit", "-m", "bl sync"])?;
            if let Err(e) = self.git_exec(&["push"]) {
                println!("Git push failed: {}", e);
            } else {
                println!("Synced successfully.");
            }
        } else {
            println!("No changes to push.");
        }

        Ok(())
    }

    /// Loads the base state from sync_base.jsonl
    fn load_base(&self) -> Result<HashMap<String, IssueExport>> {
        let path = self.repo_path.join("sync_base.jsonl");
        if !path.exists() {
            return Ok(HashMap::new());
        }
        self.load_issues_from_file(&path)
    }

    /// Loads the remote state from issues.jsonl
    fn load_remote(&self) -> Result<HashMap<String, IssueExport>> {
        let path = self.repo_path.join("issues.jsonl");
        if !path.exists() {
            return Ok(HashMap::new());
        }
        self.load_issues_from_file(&path)
    }

    /// Loads the local state from the database
    fn load_local(&self) -> Result<HashMap<String, IssueExport>> {
        let issues = self.store.list_issues()?;
        let all_deps = self.store.get_all_dependencies()?;
        let mut map = HashMap::new();

        for issue in issues {
            let deps = all_deps.get(&issue.id).map(|v| v.as_slice()).unwrap_or(&[]);
            let export = jsonl::to_issue_export(&issue, deps);
            map.insert(export.id.clone(), export);
        }
        Ok(map)
    }

    fn load_issues_from_file(&self, path: &Path) -> Result<HashMap<String, IssueExport>> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut map = HashMap::new();

        // We can reuse import_from_jsonl logic but we just want to parse into struct
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

    /// 3-way merge logic
    /// Returns the merged state as a list of IssueExport
    pub fn merge(
        &self,
        base: HashMap<String, IssueExport>,
        local: HashMap<String, IssueExport>,
        remote: HashMap<String, IssueExport>,
    ) -> Vec<IssueExport> {
        let mut merged = Vec::new();
        let mut all_ids: HashSet<String> = HashSet::new();
        all_ids.extend(base.keys().cloned());
        all_ids.extend(local.keys().cloned());
        all_ids.extend(remote.keys().cloned());

        for id in all_ids {
            let b = base.get(&id);
            let l = local.get(&id);
            let r = remote.get(&id);

            match (b, l, r) {
                (None, Some(l_issue), None) => merged.push(l_issue.clone()), // Local add
                (None, None, Some(r_issue)) => merged.push(r_issue.clone()), // Remote add
                (None, Some(l_issue), Some(r_issue)) => {
                    // Added in both concurrently
                    if l_issue.updated_at >= r_issue.updated_at {
                        merged.push(l_issue.clone());
                    } else {
                        merged.push(r_issue.clone());
                    }
                }
                (Some(_), None, None) => {} // Deleted in local and remote
                (Some(b_issue), None, Some(r_issue)) => {
                    // Local deleted, Remote kept
                    // If Remote modified it, keep Remote (resurrect). Else delete.
                    if r_issue.updated_at > b_issue.updated_at {
                        merged.push(r_issue.clone());
                    }
                }
                (Some(b_issue), Some(l_issue), None) => {
                    // Remote deleted, Local kept
                    // If Local modified it, keep Local (resurrect). Else delete.
                    if l_issue.updated_at > b_issue.updated_at {
                        merged.push(l_issue.clone());
                    }
                }
                (Some(b_issue), Some(l_issue), Some(r_issue)) => {
                    // Modified in both?
                     if l_issue.updated_at == r_issue.updated_at {
                         // No conflict, or same timestamp
                         merged.push(l_issue.clone());
                     } else if l_issue.updated_at > b_issue.updated_at && r_issue.updated_at == b_issue.updated_at {
                         // Local changed, Remote didn't
                         merged.push(l_issue.clone());
                     } else if r_issue.updated_at > b_issue.updated_at && l_issue.updated_at == b_issue.updated_at {
                         // Remote changed, Local didn't
                         merged.push(r_issue.clone());
                     } else {
                         // Both changed
                         // LWW
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
        
        merged
    }

    pub fn apply_changes(&mut self, merged: Vec<IssueExport>) -> Result<()> {
        // 1. Update DB (store)
        let mut merged_jsonl = String::new();
        for item in &merged {
            let line = serde_json::to_string(item).map_err(crate::jsonl::JsonlError::from)?;
            merged_jsonl.push_str(&line);
            merged_jsonl.push('\n');
        }
        
        // Step 1: Get all current DB IDs
        let current_issues = self.store.list_issues()?;
        let current_ids: HashSet<String> = current_issues.iter().map(|i| i.id.clone()).collect();
        let merged_ids: HashSet<String> = merged.iter().map(|i| i.id.clone()).collect();
        
        // Step 2: Delete IDs not in merged
        for id in current_ids {
            if !merged_ids.contains(&id) {
                self.store.delete_issue(&id)?;
            }
        }
        
        // Step 3: Upsert merged issues
        if !merged.is_empty() {
             crate::jsonl::import_from_jsonl(&mut self.store, std::io::Cursor::new(merged_jsonl.as_bytes()))?;
        }
        
        // 2. Write to issues.jsonl
        let issues_path = self.repo_path.join("issues.jsonl");
        let file = File::create(&issues_path)?;
        let mut writer = BufWriter::new(file);
        
        // Move merged to sorted_merged
        let mut sorted_merged = merged;
        sorted_merged.sort_by(|a, b| a.id.cmp(&b.id));
        
        for item in &sorted_merged {
             serde_json::to_writer(&mut writer, item).map_err(crate::jsonl::JsonlError::from)?;
             use std::io::Write;
             writeln!(writer)?;
        }
        
        // 3. Write to sync_base.jsonl
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
