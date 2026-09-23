//! Dev harness: applies a real update to a throwaway npm project in the temp
//! folder, once succeeding and once with a failing build to prove rollback.

use mehen_core::update::{Change, apply, plan};
use mehen_core::{IgnoreSet, scan};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let dir = std::env::temp_dir().join("mehen-apply-smoke");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let write_pkg = |build: &str| {
        std::fs::write(
            dir.join("package.json"),
            format!("{{\n  \"name\": \"smoke\",\n  \"scripts\": {{ \"build\": \"{build}\" }},\n  \"dependencies\": {{\n    \"left-pad\": \"^1.1.0\"\n  }}\n}}\n"),
        )
        .unwrap()
    };
    let npm = |args: &[&str]| std::process::Command::new("cmd").arg("/C").arg("npm").args(args).current_dir(&dir).output().unwrap();
    let locked = || {
        let lock: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("package-lock.json")).unwrap()).unwrap();
        lock["packages"]["node_modules/left-pad"]["version"].as_str().unwrap_or("?").to_string()
    };

    for (label, build) in [("passing build", "node -e \\\"process.exit(0)\\\""), ("failing build", "node -e \\\"process.exit(3)\\\"")] {
        write_pkg(build);
        let _ = std::fs::remove_file(dir.join("package-lock.json"));
        npm(&["install", "left-pad@1.1.0", "--save-exact=false", "--no-audit", "--no-fund"]);
        write_pkg(build);
        npm(&["install", "--no-audit", "--no-fund"]);
        let before_manifest = std::fs::read_to_string(dir.join("package.json")).unwrap();
        println!("== {label}: locked before = {}", locked());

        let inventory = scan(&[dir.clone()], &IgnoreSet::default());
        let project = &inventory.projects[0];
        let plan = plan(project, &[Change { from: None, name: "left-pad".into(), to: "1.3.0".into() }], |_| None).unwrap();
        let outcome = apply(&plan, true, None, |e| println!("   [{}] {}", e.state, e.label)).await;
        println!("   ok={} rolled_back={} error={:?}", outcome.ok, outcome.rolled_back, outcome.error);
        let manifest = std::fs::read_to_string(dir.join("package.json")).unwrap();
        println!("   locked after = {}; manifest restored = {}", locked(), manifest == before_manifest);
        println!("   manifest has ^1.3.0 = {}", manifest.contains("^1.3.0"));
    }
}
