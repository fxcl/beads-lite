use assert_cmd::Command;
use predicates::prelude::*;
use std::process::Command as StdCommand;
use tempfile::TempDir;

struct TestRepo {
    _tmp: TempDir,
    path: std::path::PathBuf,
}

impl TestRepo {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_path_buf();

        // Initialize a real git repo so `bl commit` has something to commit to.
        StdCommand::new("git").args(["init", "-q"]).current_dir(&path).output().unwrap();
        StdCommand::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&path)
            .output()
            .unwrap();
        StdCommand::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&path)
            .output()
            .unwrap();
        StdCommand::new("git")
            .args(["commit", "--allow-empty", "-m", "init", "-q"])
            .current_dir(&path)
            .output()
            .unwrap();

        Self { _tmp: tmp, path }
    }

    fn bl(&self) -> Command {
        #[allow(deprecated)]
        let mut cmd = Command::cargo_bin("bl").unwrap();
        cmd.current_dir(&self.path);
        cmd
    }

    fn git(&self, args: &[&str]) -> String {
        let out = StdCommand::new("git").args(args).current_dir(&self.path).output().unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    }
}

#[test]
fn commit_default_message() {
    let repo = TestRepo::new();
    repo.bl().arg("init").assert().success();
    repo.bl().arg("create").arg("Task One").assert().success();

    repo.bl().arg("commit").assert().success();

    let log = repo.git(&["log", "--oneline"]);
    assert!(log.contains("bl commit"), "expected 'bl commit' in log: {}", log);
    assert!(repo.path.join("issues.jsonl").exists());
}

#[test]
fn commit_custom_message() {
    let repo = TestRepo::new();
    repo.bl().arg("init").assert().success();
    repo.bl().arg("create").arg("Task").assert().success();

    repo.bl().arg("commit").arg("-m").arg("custom msg here").assert().success();

    let log = repo.git(&["log", "--oneline"]);
    assert!(log.contains("custom msg here"));
}

#[test]
fn commit_stdin_message() {
    let repo = TestRepo::new();
    repo.bl().arg("init").assert().success();
    repo.bl().arg("create").arg("Task").assert().success();

    repo.bl()
        .arg("commit")
        .arg("--stdin")
        .write_stdin("from stdin pipe")
        .assert()
        .success();

    let log = repo.git(&["log", "--oneline"]);
    assert!(log.contains("from stdin pipe"));
}

#[test]
fn commit_no_changes_is_noop() {
    let repo = TestRepo::new();
    repo.bl().arg("init").assert().success();
    repo.bl().arg("create").arg("Task").assert().success();

    // First commit creates issues.jsonl.
    repo.bl().arg("commit").assert().success();
    let first_log = repo.git(&["log", "--oneline"]);
    let first_count = first_log.lines().count();

    // Second commit should produce no new commit (no DB changes).
    repo.bl()
        .arg("commit")
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes"));

    let second_log = repo.git(&["log", "--oneline"]);
    let second_count = second_log.lines().count();
    assert_eq!(first_count, second_count, "no extra commit expected");
}

#[test]
fn commit_outside_ghost_repo_errors() {
    // No `git init` — just an empty dir.
    let tmp = TempDir::new().unwrap();
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("bl").unwrap();
    cmd.current_dir(tmp.path()).arg("init");
    cmd.assert().success();
    cmd.arg("commit").assert().failure();
}

#[test]
fn check_clean_when_no_db_changes() {
    let repo = TestRepo::new();
    repo.bl().arg("init").assert().success();
    repo.bl().arg("create").arg("Task").assert().success();

    // First commit populates HEAD's issues.jsonl.
    repo.bl().arg("commit").assert().success();

    // No DB changes since → --check should report clean, exit 0.
    repo.bl()
        .arg("commit")
        .arg("--check")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("clean"));
}

#[test]
fn check_dirty_when_db_changed() {
    let repo = TestRepo::new();
    repo.bl().arg("init").assert().success();
    repo.bl().arg("create").arg("Task").assert().success();
    repo.bl().arg("commit").assert().success();

    // Add a new issue but don't commit → DB diverges from HEAD.
    repo.bl().arg("create").arg("New task").assert().success();

    repo.bl()
        .arg("commit")
        .arg("--check")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("would commit"));
}

#[test]
fn check_json_output() {
    let repo = TestRepo::new();
    repo.bl().arg("init").assert().success();
    repo.bl().arg("create").arg("Task").assert().success();

    // Clean state (HEAD has no issues.jsonl yet, DB has 1 issue → dirty).
    repo.bl()
        .arg("commit")
        .arg("--check")
        .arg("--json")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\"dirty\":true"));

    // After actual commit, clean.
    repo.bl().arg("commit").assert().success();
    repo.bl()
        .arg("commit")
        .arg("--check")
        .arg("--json")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("\"dirty\":false"));
}

#[test]
fn check_does_not_write_files() {
    let repo = TestRepo::new();
    repo.bl().arg("init").assert().success();
    repo.bl().arg("create").arg("Task").assert().success();

    // First commit creates issues.jsonl + commit. Snapshot HEAD.
    repo.bl().arg("commit").assert().success();
    let head_before = repo.git(&["rev-parse", "HEAD"]);
    let jsonl_before = std::fs::read_to_string(repo.path.join("issues.jsonl")).unwrap();

    // --check must not touch anything.
    repo.bl().arg("commit").arg("--check").assert().code(0);
    repo.bl().arg("commit").arg("--check").arg("--json").assert().code(0);

    let head_after = repo.git(&["rev-parse", "HEAD"]);
    assert_eq!(head_before, head_after, "HEAD must not advance");
    let jsonl_after = std::fs::read_to_string(repo.path.join("issues.jsonl")).unwrap();
    assert_eq!(jsonl_before, jsonl_after, "issues.jsonl must not be rewritten");
}
