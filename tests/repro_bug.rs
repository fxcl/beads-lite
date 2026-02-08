use assert_cmd::Command;
use regex::Regex;
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

#[test]
fn test_repro_bug_ready_shows_blocked() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();

    // 1. Create DB (uuve)
    let out_db = String::from_utf8(env.cmd().arg("create").arg("DB").assert().success().get_output().stdout.clone()).unwrap();
    let id_db = extract_id(&out_db);

    // 2. Create API (h570)
    let out_api = String::from_utf8(env.cmd().arg("create").arg("API").assert().success().get_output().stdout.clone()).unwrap();
    let id_api = extract_id(&out_api);

    // 3. Create UI (imhq)
    let out_ui = String::from_utf8(env.cmd().arg("create").arg("UI").assert().success().get_output().stdout.clone()).unwrap();
    let id_ui = extract_id(&out_ui);

    // 4. Set Dependencies
    // API blocked by DB
    env.cmd()
        .arg("update")
        .arg(&id_api)
        .arg("--blocked-by")
        .arg(&id_db)
        .assert()
        .success();
    // UI blocked by API
    env.cmd()
        .arg("update")
        .arg(&id_ui)
        .arg("--blocked-by")
        .arg(&id_api)
        .assert()
        .success();

    // 5. Check Ready
    let output = env.cmd().arg("ready").assert().success().get_output().stdout.clone();
    let ready_str = String::from_utf8(output).unwrap();

    println!("Ready output:\n{}", ready_str);

    // Should ONLY see DB
    assert!(ready_str.contains("DB"), "DB should be visible");
    assert!(!ready_str.contains("API"), "API should be HIDDEN");
    assert!(!ready_str.contains("UI"), "UI should be HIDDEN");
}
