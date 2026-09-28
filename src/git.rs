//! Git shell-out helpers used by `commit` and `sync`.

use std::path::Path;
use std::process::{Command, Output};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("git {args:?} exited with status {code}: {stderr}")]
    CommandFailed { args: Vec<String>, code: i32, stderr: String },
    #[error("not a git repository: {0}")]
    NotARepository(String),
    #[error("push race (transient) after {attempts} attempt(s): {stderr}")]
    PushRace { attempts: u32, stderr: String },
}

/// Outcome of `commit_snapshot`. `false` means nothing was committed
/// (no staged changes).
pub type CommitOutcome = bool;

/// Result of a push attempt with retry budget exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOutcome {
    Pushed,
    Skipped,
}

/// Runs `git <args>` in `repo`. Returns Err on non-zero exit or spawn failure.
pub fn run(repo: &Path, args: &[&str]) -> Result<(), GitError> {
    let output = Command::new("git").current_dir(repo).args(args).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GitError::CommandFailed {
            args: args.iter().map(|s| s.to_string()).collect(),
            code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

/// Runs `git <args>` and returns stdout. Treats non-zero as an error.
pub fn run_capture(repo: &Path, args: &[&str]) -> Result<String, GitError> {
    let output = Command::new("git").current_dir(repo).args(args).output()?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            args: args.iter().map(|s| s.to_string()).collect(),
            code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string().to_string())
}

/// Returns true if the repo is inside a git working tree.
pub fn is_repository(repo: &Path) -> bool {
    Command::new("git")
        .current_dir(repo)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Returns the contents of `<path>` at HEAD, or `None` if the file is not
/// tracked at HEAD (or any other non-fatal `git show` failure).
pub fn head_file(repo: &Path, path: &str) -> Option<String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(["show", &format!("HEAD:{}", path)])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

/// Returns the current git user's name (preferred for comment authorship).
/// Falls back to system user, then to `"unknown"`.
pub fn current_user() -> String {
    if let Ok(out) = Command::new("git").args(["config", "user.name"]).output() {
        if out.status.success() {
            let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }
    if let Ok(name) = std::env::var("USER") {
        if !name.is_empty() {
            return name;
        }
    }
    "unknown".to_string()
}

/// Returns true if `git diff --cached` is non-empty (i.e., there are staged
/// changes ready to commit).
pub fn has_staged_changes(repo: &Path) -> Result<bool, GitError> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(["diff", "--cached", "--quiet"])
        .output()?;
    // exit 0 = no diff (clean), exit 1 = diff present (staged)
    Ok(!output.status.success() && output.status.code() == Some(1))
}

/// Stages `files` (relative paths) and commits with `msg`. Returns
/// `CommitOutcome::false` when there are no staged changes.
pub fn commit_snapshot(repo: &Path, files: &[&str], msg: &str) -> Result<CommitOutcome, GitError> {
    if !is_repository(repo) {
        return Err(GitError::NotARepository(repo.display().to_string()));
    }
    if files.is_empty() {
        return Err(GitError::CommandFailed {
            args: vec!["commit_snapshot".into()],
            code: -1,
            stderr: "no files to stage".to_string(),
        });
    }
    for f in files {
        run(repo, &["add", "--", f])?;
    }
    if !has_staged_changes(repo)? {
        return Ok(false);
    }
    run(repo, &["commit", "-m", msg])?;
    Ok(true)
}

/// Pushes with retry on transient `push race` failures (remote ahead / fast-forward
/// required / stale info etc). Non-race failures return immediately.
pub fn push_with_retry(repo: &Path, attempts: u32) -> Result<PushOutcome, GitError> {
    let attempts = attempts.max(1);
    let mut last_stderr = String::new();
    for _n in 1..=attempts {
        let output: Output = Command::new("git").current_dir(repo).args(["push"]).output()?;
        if output.status.success() {
            return Ok(PushOutcome::Pushed);
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        last_stderr = stderr.clone();
        if !is_push_race(&stderr) {
            return Err(GitError::CommandFailed {
                args: vec!["push".to_string()],
                code: output.status.code().unwrap_or(-1),
                stderr,
            });
        }
    }
    Err(GitError::PushRace {
        attempts,
        stderr: last_stderr,
    })
}

/// Returns true if `stderr` matches the well-known push-race transient
/// signatures (remote ahead, fast-forward required, stale info, fetch first,
/// contains work that you do not have).
pub fn is_push_race(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    let needles = [
        "behind",
        "fast-forward",
        "fast forward",
        "not an ancestor",
        "not ancestor",
        "stale info",
        "fetch first",
        "contains work that you do not have",
    ];
    needles.iter().any(|n| lower.contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_race_detection() {
        assert!(is_push_race(
            "error: failed to push some refs: remote contains work that you do not have"
        ));
        assert!(is_push_race(
            "hint: Updates were rejected because the tip of your current branch is behind"
        ));
        assert!(is_push_race("! [rejected] (stale info)"));
        assert!(is_push_race("non-fast-forward"));
        // Non-race failures should NOT match.
        assert!(!is_push_race("fatal: could not read username for 'https://x'"));
        assert!(!is_push_race("error: cannot push because you have unmerged files"));
        assert!(!is_push_race(""));
    }
}
