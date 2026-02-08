use assert_cmd::Command;
use predicates::prelude::*;
use regex::Regex;
use tempfile::TempDir;

// Helper to run CLI in a clean temp directory
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

// Extractor helper
fn extract_id(output: &str) -> String {
    let re = Regex::new(r"bl-[a-z0-9]{4}").unwrap();
    re.find(output).map(|m| m.as_str().to_string()).unwrap_or_default()
}

#[test]
fn test_init() {
    let env = TestEnv::new();
    env.cmd()
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized beads-lite"));

    assert!(env.path.join(".beads-lite/beads.db").exists());
}

#[test]
fn test_create_basic() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();

    let assert = env.cmd().arg("create").arg("Test Task").assert().success();
    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(output.contains("Created bl-"));
    assert!(output.contains("Test Task"));
}

#[test]
fn test_list_empty() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();

    env.cmd()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("No issues found"));
}

#[test]
fn test_create_and_list() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();
    env.cmd().arg("create").arg("Task A").assert().success();
    env.cmd().arg("create").arg("Task B").assert().success();

    env.cmd()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("Task A"))
        .stdout(predicate::str::contains("Task B"));
}

#[test]
fn test_show() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();

    let assert = env.cmd().arg("create").arg("My Show Task").assert().success();
    let id = extract_id(&String::from_utf8(assert.get_output().stdout.clone()).unwrap());

    env.cmd()
        .arg("show")
        .arg(&id)
        .assert()
        .success()
        .stdout(predicate::str::contains(&id))
        .stdout(predicate::str::contains("My Show Task"))
        .stdout(predicate::str::contains("open"));
}

#[test]
fn test_update() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();

    let assert = env.cmd().arg("create").arg("Task").assert().success();
    let id = extract_id(&String::from_utf8(assert.get_output().stdout.clone()).unwrap());

    env.cmd()
        .arg("update")
        .arg(&id)
        .arg("--title")
        .arg("Updated Title")
        .arg("--status")
        .arg("in_progress")
        .assert()
        .success();

    env.cmd()
        .arg("show")
        .arg(&id)
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated Title"))
        .stdout(predicate::str::contains("in_progress"));
}

#[test]
fn test_close() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();

    let assert = env.cmd().arg("create").arg("Task").assert().success();
    let id = extract_id(&String::from_utf8(assert.get_output().stdout.clone()).unwrap());

    env.cmd().arg("close").arg(&id).assert().success();

    env.cmd()
        .arg("show")
        .arg(&id)
        .assert()
        .success()
        .stdout(predicate::str::contains("closed"))
        .stdout(predicate::str::contains("done"));
}

#[test]
fn test_blocking_chain() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();

    // Create A
    let out_a = String::from_utf8(env.cmd().arg("create").arg("Task A").assert().success().get_output().stdout.clone()).unwrap();
    let id_a = extract_id(&out_a);

    // Create B blocked by A
    let out_b = String::from_utf8(
        env.cmd()
            .arg("create")
            .arg("Task B")
            .arg("--blocked-by")
            .arg(&id_a)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    let id_b = extract_id(&out_b);

    // Create C blocked by B
    let out_c = String::from_utf8(
        env.cmd()
            .arg("create")
            .arg("Task C")
            .arg("--blocked-by")
            .arg(&id_b)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    let _id_c = extract_id(&out_c);

    // Check Ready - only A should be visible
    let ready_1 = String::from_utf8(env.cmd().arg("ready").assert().success().get_output().stdout.clone()).unwrap();
    assert!(ready_1.contains("Task A"));
    assert!(!ready_1.contains("Task B"));
    assert!(!ready_1.contains("Task C"));

    // Close A
    env.cmd().arg("close").arg(&id_a).assert().success();

    // Check Ready - now B should be visible
    let ready_2 = String::from_utf8(env.cmd().arg("ready").assert().success().get_output().stdout.clone()).unwrap();
    assert!(!ready_2.contains("Task A")); // Closed
    assert!(ready_2.contains("Task B"));
    assert!(!ready_2.contains("Task C"));

    // Close B
    env.cmd().arg("close").arg(&id_b).assert().success();

    // Check Ready - now C should be visible
    let ready_3 = String::from_utf8(env.cmd().arg("ready").assert().success().get_output().stdout.clone()).unwrap();
    assert!(!ready_3.contains("Task A"));
    assert!(!ready_3.contains("Task B"));
    assert!(ready_3.contains("Task C"));
}

#[test]
fn test_tree_output() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();

    let out_a = String::from_utf8(env.cmd().arg("create").arg("Top").assert().success().get_output().stdout.clone()).unwrap();
    let id_a = extract_id(&out_a);

    let out_b = String::from_utf8(
        env.cmd()
            .arg("create")
            .arg("Child")
            .arg("--blocked-by")
            .arg(&id_a)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    let _id_b = extract_id(&out_b);

    let tree = String::from_utf8(env.cmd().arg("list").arg("--tree").assert().success().get_output().stdout.clone()).unwrap();

    // Check for box drawing character
    assert!(tree.contains("└──"));
    // A should be printed before B in the visual hierarchy (or at least structure is present)
    assert!(tree.contains("Top"));
    assert!(tree.contains("Child"));
}
