use assert_cmd::Command;
use predicates::prelude::*;
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
fn test_repro_tree_structure() {
    let env = TestEnv::new();
    env.cmd().arg("init").assert().success();

    // 1. Create DB (A)
    let out_a = String::from_utf8(env.cmd().arg("create").arg("DB").assert().success().get_output().stdout.clone()).unwrap();
    let id_a = extract_id(&out_a);

    // 2. Create API (B)
    let out_b = String::from_utf8(env.cmd().arg("create").arg("API").assert().success().get_output().stdout.clone()).unwrap();
    let id_b = extract_id(&out_b);

    // 3. Create UI (C)
    let out_c = String::from_utf8(env.cmd().arg("create").arg("UI").assert().success().get_output().stdout.clone()).unwrap();
    let id_c = extract_id(&out_c);

    // 4. Link: C -> B -> A
    env.cmd().arg("update").arg(&id_b).arg("--blocked-by").arg(&id_a).assert().success();
    env.cmd().arg("update").arg(&id_c).arg("--blocked-by").arg(&id_b).assert().success();

    // 5. List Tree
    let output = env.cmd().arg("list").arg("--tree").assert().success().get_output().stdout.clone();
    let tree_str = String::from_utf8(output).unwrap();
    
    println!("Tree output:\n{}", tree_str);

    // Should see structure chars
    // The previous analysis suggests '└──' or '├──' characters.
    // If flat, we see just lines of tasks.
    
    assert!(tree_str.contains("└──"), "Tree output should contain branch characters");
    // Verify hierarchy order roughly (A then B then C)
    let idx_a = tree_str.find("DB").unwrap();
    let idx_b = tree_str.find("API").unwrap();
    let idx_c = tree_str.find("UI").unwrap();
    
    assert!(idx_a < idx_b, "DB should be printed before API");
    assert!(idx_b < idx_c, "API should be printed before UI");
}
