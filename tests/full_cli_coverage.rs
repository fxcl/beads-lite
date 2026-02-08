use assert_cmd::Command;
use predicates::prelude::*;
use regex::Regex;
use std::fs;
use tempfile::TempDir;

struct TestEnv {
    _temp: TempDir,
    path: std::path::PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let path = temp.path().to_path_buf();
        Self { _temp: temp, path }
    }

    fn cmd(&self) -> Command {
        #[allow(deprecated)]
        let mut cmd = Command::cargo_bin("bl").unwrap();
        cmd.current_dir(&self.path);
        cmd
    }
}

fn extract_id(output: &str) -> String {
    let re = Regex::new(r"bl-[a-z0-9]{4}").unwrap();
    re.find(output).map(|m| m.as_str().to_string()).unwrap_or_default()
}

// 1. INIT
#[test]
fn test_cmd_init() {
    let env = TestEnv::new();
    env.cmd()
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized"));
    assert!(env.path.join(".beads-lite/beads.db").exists());
    // Idempotency check
    env.cmd().arg("init").assert().success();
}

// 2. CREATE (All flags)
#[test]
fn test_cmd_create_full() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();

    let output = env
        .cmd()
        .arg("create")
        .arg("Complex Task")
        .arg("--description")
        .arg("Detailed description")
        .arg("--priority")
        .arg("0")
        .arg("--type")
        .arg("bug")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let out_str = String::from_utf8(output).unwrap();
    let id = extract_id(&out_str);
    assert!(!id.is_empty());

    // Verify
    env.cmd().arg("show").arg(&id)
        .assert()
        .success()
        .stdout(predicate::str::contains("Complex Task"))
        .stdout(predicate::str::contains("Detailed description"))
        .stdout(predicate::str::contains("bug")) // P0 is usually high priority, check CLI output formatting later
        .stdout(predicate::str::contains("P0"));
}

// 3. LIST (Filters)
#[test]
fn test_cmd_list_filters() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();
    // Create Task A, then update status
    let out = env.cmd().arg("create").arg("Task A").assert().success().get_output().stdout.clone();
    let id_a = extract_id(&String::from_utf8(out).unwrap());

    // Create does not support --status, must use update
    env.cmd()
        .arg("update")
        .arg(&id_a)
        .arg("--status")
        .arg("in_progress")
        .assert()
        .success();

    let out = env
        .cmd()
        .arg("create")
        .arg("Bug B")
        .arg("--type")
        .arg("bug")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let _id_b = extract_id(&String::from_utf8(out).unwrap());

    // Filter by status
    env.cmd()
        .arg("list")
        .arg("--status")
        .arg("in_progress")
        .assert()
        .success()
        .stdout(predicate::str::contains("Task A"))
        .stdout(predicate::str::contains("Bug B").not());

    // Filter by type
    env.cmd()
        .arg("list")
        .arg("--type")
        .arg("bug")
        .assert()
        .success()
        .stdout(predicate::str::contains("Bug B"))
        .stdout(predicate::str::contains("Task A").not());
}

// 4. SHOW (JSON)
#[test]
fn test_cmd_show_json() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();
    let out = env
        .cmd()
        .arg("create")
        .arg("JSON Task")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let id = extract_id(&String::from_utf8(out).unwrap());

    let json_out = env
        .cmd()
        .arg("show")
        .arg(&id)
        .arg("--json")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json_str = String::from_utf8(json_out).unwrap();

    assert!(json_str.contains("\"title\":\"JSON Task\""));
    assert!(json_str.contains("\"id\":\"bl-"));
}

// 5. UPDATE (All fields)
#[test]
fn test_cmd_update_all() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();
    let out = env.cmd().arg("create").arg("Old").assert().success().get_output().stdout.clone();
    let id = extract_id(&String::from_utf8(out).unwrap());

    env.cmd().arg("update").arg(&id)
        .arg("--title").arg("New")
        .arg("--description").arg("New Desc")
        .arg("--priority").arg("4")
        .arg("--type").arg("epic")
        .arg("--status").arg("closed") // This might close it?
        .assert().success();

    env.cmd()
        .arg("show")
        .arg(&id)
        .assert()
        .success()
        .stdout(predicate::str::contains("New"))
        .stdout(predicate::str::contains("New Desc"))
        .stdout(predicate::str::contains("P4"))
        .stdout(predicate::str::contains("epic"))
        .stdout(predicate::str::contains("closed"));
}

// 6. DELETE
#[test]
fn test_cmd_delete() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();
    let out = env.cmd().arg("create").arg("Temp").assert().success().get_output().stdout.clone();
    let id = extract_id(&String::from_utf8(out).unwrap());

    // Without confirm
    env.cmd().arg("delete").arg(&id).assert().failure();

    // With confirm
    env.cmd().arg("delete").arg(&id).arg("--confirm").assert().success();

    // Verify gone
    env.cmd().arg("show").arg(&id).assert().failure();
}

// 7. CLOSE
#[test]
fn test_cmd_close_resolutions() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();

    let out = env.cmd().arg("create").arg("Task").assert().success().get_output().stdout.clone();
    let id = extract_id(&String::from_utf8(out).unwrap());

    env.cmd()
        .arg("close")
        .arg(&id)
        .arg("--resolution")
        .arg("wontfix")
        .assert()
        .success();

    env.cmd()
        .arg("show")
        .arg(&id)
        .assert()
        .success()
        .stdout(predicate::str::contains("closed"))
        .stdout(predicate::str::contains("wontfix"));
}

// 8. READY (Blocking logic)
#[test]
fn test_cmd_ready_complex() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();

    let out_a = String::from_utf8(env.cmd().arg("create").arg("A").assert().success().get_output().stdout.clone()).unwrap();
    let id_a = extract_id(&out_a);

    let out_b = String::from_utf8(env.cmd().arg("create").arg("B").assert().success().get_output().stdout.clone()).unwrap();
    let id_b = extract_id(&out_b);

    // B blocked by A via update
    env.cmd().arg("update").arg(&id_b).arg("--blocked-by").arg(&id_a).assert().success();

    // Verify Ready
    let ready = String::from_utf8(env.cmd().arg("ready").assert().success().get_output().stdout.clone()).unwrap();
    assert!(ready.contains("A"));
    assert!(!ready.contains("B"));

    // Verify remove blocker
    env.cmd().arg("update").arg(&id_b).arg("--unblock").arg(&id_a).assert().success();

    // Now B should be ready
    let ready = String::from_utf8(env.cmd().arg("ready").assert().success().get_output().stdout.clone()).unwrap();
    assert!(ready.contains("A"));
    assert!(ready.contains("B"));
}

// 9. IMPORT/EXPORT
#[test]
fn test_cmd_import_export() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();
    env.cmd().arg("create").arg("ExportMe").assert().success();

    let export_file = env.path.join("dump.jsonl");
    env.cmd().arg("export").arg(export_file.to_str().unwrap()).assert().success();

    assert!(export_file.exists());

    // Nuke database
    fs::remove_file(env.path.join(".beads-lite/beads.db")).unwrap();

    // Re-init and import
    env.cmd().arg("init").assert().success();
    env.cmd().arg("import").arg(export_file.to_str().unwrap()).assert().success();

    // Verify
    env.cmd()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("ExportMe"));
}

// 10. ONBOARD
#[test]
fn test_cmd_onboard() {
    let env = TestEnv::new();
    let output = String::from_utf8(env.cmd().arg("onboard").assert().success().get_output().stdout.clone()).unwrap();
    // Match a string we definitely see in the file view
    assert!(output.contains("This project uses beads-lite"));
}

// 11. VERSION
#[test]
fn test_cmd_version() {
    let env = TestEnv::new();
    env.cmd()
        .arg("version")
        .assert()
        .success()
        .stdout(predicate::str::contains("bl version"));
    env.cmd()
        .arg("-v")
        .assert()
        .success()
        .stdout(predicate::str::contains("bl version"));
    env.cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("bl version"));
}
