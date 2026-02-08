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
fn test_tree_with_closed_parents() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();

    // 1. Create Parent and Child
    let out_p = String::from_utf8(env.cmd().arg("create").arg("Parent").assert().success().get_output().stdout.clone()).unwrap();
    let id_p = extract_id(&out_p);

    let out_c = String::from_utf8(env.cmd().arg("create").arg("Child").assert().success().get_output().stdout.clone()).unwrap();
    let id_c = extract_id(&out_c);

    // 2. Link: Child blocked by Parent
    env.cmd().arg("update").arg(&id_c).arg("--blocked-by").arg(&id_p).assert().success();

    // 3. Verify tree shows structure initially (Parent is OPEN)
    let output = env.cmd().arg("list").arg("--tree").assert().success().get_output().stdout.clone();
    let tree_open = String::from_utf8(output).unwrap();
    assert!(tree_open.contains("└──"), "Tree should show structure when parent is OPEN");

    // 4. Close Parent
    env.cmd().arg("close").arg(&id_p).assert().success();

    // 5. Check Tree again (Parent is CLOSED)
    let output = env.cmd().arg("list").arg("--tree").assert().success().get_output().stdout.clone();
    let tree_closed = String::from_utf8(output).unwrap();
    println!("Tree closed output:\n{}", tree_closed);

    // BUG: If this assertion FAILS, it means the structure is missing (flat list).
    // We expect the tree to PERSIST even if parent is closed, because `bl list` shows history.
    assert!(tree_closed.contains("└──"), "Tree should STILL show structure when parent is CLOSED");
}
