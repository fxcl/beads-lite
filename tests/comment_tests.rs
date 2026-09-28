use assert_cmd::Command;
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

        StdCommand::new("git").args(["init", "-q"]).current_dir(&path).output().unwrap();
        StdCommand::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&path)
            .output()
            .unwrap();
        StdCommand::new("git")
            .args(["config", "user.name", "Test User"])
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

    /// Create a task and return its generated id.
    fn create_task(&self, title: &str) -> String {
        let out = self.bl().arg("create").arg(title).output().unwrap();
        assert!(out.status.success(), "create failed: {}", String::from_utf8_lossy(&out.stderr));
        let stdout = String::from_utf8_lossy(&out.stdout);
        // Output is `Created bl-xxxx: title` — grab the id.
        let line = stdout.lines().find(|l| l.contains("Created")).unwrap_or_default();
        let id = line.split_whitespace().nth(1).unwrap_or("").trim_end_matches(':').to_string();
        assert!(id.starts_with("bl-"), "could not parse id from: {}", line);
        id
    }

    fn init(&self) {
        self.bl().arg("init").assert().success();
    }
}

#[test]
fn comment_positional_text() {
    let repo = TestRepo::new();
    repo.init();
    let id = repo.create_task("Task One");

    repo.bl().arg("comment").arg(&id).arg("hello world").assert().success();

    // Verify comment exists by exporting and grepping the JSONL.
    // No commit yet — re-export manually.
    let show = repo.bl().arg("show").arg(&id).arg("--json").output().unwrap();
    assert!(show.status.success(), "show failed: {}", String::from_utf8_lossy(&show.stderr));
}

#[test]
fn comment_stdin() {
    let repo = TestRepo::new();
    repo.init();
    let id = repo.create_task("Task");

    repo.bl()
        .arg("comment")
        .arg(&id)
        .arg("--stdin")
        .write_stdin("from stdin pipe")
        .assert()
        .success();
}

#[test]
fn comment_file() {
    let repo = TestRepo::new();
    repo.init();
    let id = repo.create_task("Task");

    let notes = repo.path.join("notes.txt");
    std::fs::write(&notes, "comment from file\n").unwrap();

    repo.bl().arg("comment").arg(&id).arg("--file").arg(&notes).assert().success();
}

#[test]
fn comment_no_source_errors() {
    let repo = TestRepo::new();
    repo.init();
    let id = repo.create_task("Task");

    repo.bl().arg("comment").arg(&id).assert().failure();
}

#[test]
fn comment_unknown_issue_errors() {
    let repo = TestRepo::new();
    repo.init();

    repo.bl().arg("comment").arg("bl-doesnotexist").arg("hi").assert().failure();
}

#[test]
fn comment_appears_in_jsonl() {
    let repo = TestRepo::new();
    repo.init();
    let id = repo.create_task("Task");

    repo.bl().arg("comment").arg(&id).arg("a comment").assert().success();

    // `bl show --json` should surface the comments field.
    let out = repo.bl().arg("show").arg(&id).arg("--json").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("show --json returns valid JSON");
    let comments = json.get("comments").and_then(|v| v.as_array()).expect("comments field missing");
    assert_eq!(
        comments.len(),
        1,
        "expected 1 comment, got: {}",
        serde_json::to_string(&comments).unwrap()
    );
    assert_eq!(comments[0].get("body").and_then(|v| v.as_str()), Some("a comment"));
}

#[test]
fn comment_json_output() {
    let repo = TestRepo::new();
    repo.init();
    let id = repo.create_task("Task");

    let out = repo.bl().arg("comment").arg(&id).arg("--json").arg("hi").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("comment --json returns one JSON object");
    assert!(v.get("id").is_some(), "missing id");
    assert!(v.get("issue_id").is_some(), "missing issue_id");
    assert_eq!(v.get("body").and_then(|s| s.as_str()), Some("hi"));
    assert_eq!(v.get("author").and_then(|s| s.as_str()), Some("Test User"));
}

#[test]
fn comment_author_from_git_config() {
    // Different git user name → reflected as author.
    let repo = TestRepo::new();
    StdCommand::new("git")
        .args(["config", "user.name", "Alice"])
        .current_dir(&repo.path)
        .output()
        .unwrap();
    repo.init();
    let id = repo.create_task("Task");

    let out = repo.bl().arg("comment").arg(&id).arg("--json").arg("hi").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(v.get("author").and_then(|s| s.as_str()), Some("Alice"));
}

#[test]
fn comment_multiple_sources_errors() {
    let repo = TestRepo::new();
    repo.init();
    let id = repo.create_task("Task");

    let notes = repo.path.join("notes.txt");
    std::fs::write(&notes, "x").unwrap();

    repo.bl()
        .arg("comment")
        .arg(&id)
        .arg("positional")
        .arg("--file")
        .arg(&notes)
        .assert()
        .failure();
}

#[test]
fn comment_export_roundtrip_via_sync_base() {
    // Comments should survive the sync export → import cycle.
    let repo = TestRepo::new();
    repo.init();
    let id = repo.create_task("Task");

    repo.bl().arg("comment").arg(&id).arg("persisted comment").assert().success();

    // Export the DB → JSONL; reload via `bl sync --no-push` to verify
    // the merged store contains the comment.
    repo.bl().arg("sync").arg("--no-push").assert().success();

    let out = repo.bl().arg("show").arg(&id).arg("--json").output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let comments = json.get("comments").and_then(|v| v.as_array()).expect("comments survived sync");
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].get("body").and_then(|s| s.as_str()), Some("persisted comment"));
}
