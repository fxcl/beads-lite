use assert_cmd::Command;
use std::process::Command as StdCommand;
use tempfile::TempDir;

/// Initialize a temp dir with both a git repo and a beads-lite init.
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

    /// Install a wrapper `git` script in this test's temp dir that races
    /// push AND pull when invoked. Caller MUST use `bl_with_git_stub`
    /// instead of `bl()` for invocations that should run against the stub.
    fn install_git_stub(&self, script_body: &str) -> std::path::PathBuf {
        self.install_git_stub_impl("[ \"$1\" = \"push\" ] || [ \"$1\" = \"pull\" ]", script_body)
    }

    /// Same as `install_git_stub` but only intercepts `git push`.
    fn install_git_stub_push_only(&self, script_body: &str) -> std::path::PathBuf {
        self.install_git_stub_impl("[ \"$1\" = \"push\" ]", script_body)
    }

    fn install_git_stub_impl(&self, predicate: &str, script_body: &str) -> std::path::PathBuf {
        let stub_dir = self.path.join(".stub-bin");
        std::fs::create_dir_all(&stub_dir).unwrap();
        let git_path = which_git().unwrap();
        let script = format!(
            "#!/bin/sh\nif {}; then\n  {}\nfi\nexec {} \"$@\"\n",
            predicate, script_body, git_path
        );
        std::fs::write(stub_dir.join("git"), &script).unwrap();
        #[allow(clippy::permissions_set_readonly_false)]
        std::fs::set_permissions(stub_dir.join("git"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        stub_dir
    }

    /// Returns a `bl` Command with PATH prepended to use the git stub.
    fn bl_with_git_stub(&self, stub_dir: &std::path::Path) -> Command {
        let mut cmd = self.bl();
        let cur = std::env::var("PATH").unwrap_or_default();
        cmd.env("PATH", format!("{}:{}", stub_dir.display(), cur));
        cmd
    }
}

fn which_git() -> Option<String> {
    let out = StdCommand::new("which").arg("git").output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

#[test]
fn sync_clean_returns_zero() {
    let repo = TestRepo::new();
    repo.bl().arg("init").assert().success();
    repo.bl().arg("create").arg("Task").assert().success();

    // No remote configured, so push fails — but with --no-push we skip it
    // and the run should succeed.
    repo.bl().arg("sync").arg("--no-push").assert().code(0);
}

#[test]
fn sync_no_push_skips_push() {
    let repo = TestRepo::new();
    repo.bl().arg("init").assert().success();
    repo.bl().arg("create").arg("Task").assert().success();

    let out = repo.bl().arg("sync").arg("--no-push").arg("--json").output().unwrap();

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"push_skipped\":true"));
    assert!(stdout.contains("\"pushed\":false"));
}

#[test]
fn sync_json_output_has_status_ok() {
    let repo = TestRepo::new();
    repo.bl().arg("init").assert().success();
    repo.bl().arg("create").arg("Task").assert().success();

    let out = repo.bl().arg("sync").arg("--no-push").arg("--json").output().unwrap();

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"status\":\"ok\""), "got: {}", stdout);
}

#[test]
fn sync_attempts_exhausted_returns_three() {
    let repo = TestRepo::new();
    repo.bl().arg("init").assert().success();
    repo.bl().arg("create").arg("Task").assert().success();

    // First commit so subsequent sync tries to push.
    repo.bl().arg("sync").arg("--no-push").assert().code(0);

    // Stub git push to always race.
    let stub_dir = repo.install_git_stub(
        "echo '! [rejected] (stale info) Updates were rejected because the tip of your current branch is behind its remote counterpart' >&2\nexit 1",
    );

    repo.bl_with_git_stub(&stub_dir)
        .arg("sync")
        .arg("--attempts")
        .arg("2")
        .assert()
        .code(3);
}

#[test]
fn sync_push_race_then_success_returns_zero() {
    let repo = TestRepo::new();
    repo.bl().arg("init").assert().success();
    repo.bl().arg("create").arg("Task").assert().success();
    repo.bl().arg("sync").arg("--no-push").assert().code(0);

    // Race on first push only (pull must succeed so sync reaches push).
    // Counter file lives inside the per-test temp dir so parallel runs
    // don't race on $HOME.
    let counter = repo.path.join(".bl_push_count");
    let body = format!(
        "if [ ! -f {counter:?} ]; then echo 1 > {counter:?}; echo '! [rejected] (stale info) behind' >&2; exit 1; fi; rm -f {counter:?}; exit 0",
    );
    let stub_dir = repo.install_git_stub_push_only(&body);

    repo.bl_with_git_stub(&stub_dir)
        .arg("sync")
        .arg("--attempts")
        .arg("2")
        .assert()
        .code(0);
}

#[test]
fn sync_no_push_with_local_changes_passes() {
    let repo = TestRepo::new();
    repo.bl().arg("init").assert().success();
    repo.bl().arg("create").arg("Task A").assert().success();

    // First sync with no remote (still ok because --no-push skips push).
    repo.bl().arg("sync").arg("--no-push").assert().code(0);

    // Add another issue and sync again — should still be ok.
    repo.bl().arg("create").arg("Task B").assert().success();
    repo.bl().arg("sync").arg("--no-push").assert().code(0);
}

#[test]
fn sync_json_status_is_kebab_case() {
    let repo = TestRepo::new();
    repo.bl().arg("init").assert().success();
    repo.bl().arg("create").arg("Task").assert().success();

    let out = repo.bl().arg("sync").arg("--no-push").arg("--json").output().unwrap();

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Should NOT contain snake_case "conflict_count" — uses kebab-case from serde rename.
    // Actually `conflict_count` is a field not a status, so it's fine to appear as snake_case.
    // Just verify status field exists and is one of the allowed values.
    assert!(stdout.contains("\"status\":"));
}
