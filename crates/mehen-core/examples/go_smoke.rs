//! Dev harness: updates one module in a throwaway copy of a real Go project
//! with the batch runner (go mod tidy, go build, go test), then tries a version
//! that does not exist to prove the files are put back.
//! `cargo run --example go_smoke -- C:\code\audplexus github.com/ulikunitz/xz v0.5.17 [--no-checks]`

use std::path::{Path, PathBuf};

use mehen_core::batch::{BatchOptions, run};
use mehen_core::update::{self, Change, plan};
use mehen_core::{Ecosystem, IgnoreSet, scan};

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let name = entry.file_name();
        if name == ".git" || name == "node_modules" {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (source, module, to) = (PathBuf::from(&args[0]), &args[1], &args[2]);
    let dir = std::env::temp_dir().join("mehen-go-smoke");
    let _ = std::fs::remove_dir_all(&dir);
    copy_tree(&source, &dir);

    for (label, version) in [("real version", to.as_str()), ("missing version", "v9.9.9")] {
        let inventory = scan(&[dir.clone()], &IgnoreSet::default());
        let project = inventory.projects.iter().find(|p| p.ecosystem == Ecosystem::Go).expect("a Go project");
        let dep = project.dependencies.iter().find(|d| &d.name == module).expect("the module");
        let mut dep_project = project.clone();
        for d in &mut dep_project.dependencies {
            d.current = d.installed.clone();
        }
        let before = std::fs::read_to_string(dir.join("go.mod")).unwrap();
        let plan = plan(&dep_project, &[Change { from: None, name: dep.name.clone(), to: version.into() }], |_| None).unwrap();
        println!("== {label}: {} {} -> {version}", dep.name, dep.requested);
        let options = BatchOptions { checks: !args.iter().any(|a| a == "--no-checks"), commit: false, parallel: 1, stop_on_failure: true };
        let outcomes = run(vec![plan], options, |s| async move { update::run_step(&s).await }, |e| println!("   [{:?}] {}", e.state, e.label.unwrap_or_default())).await;
        let o = &outcomes[0];
        for s in &o.steps {
            println!("   step {:<16} ok={} {} ms", s.label, s.ok, s.ms);
        }
        let after = std::fs::read_to_string(dir.join("go.mod")).unwrap();
        println!("   ok={} rolled_back={} error={:?}", o.ok, o.rolled_back, o.error);
        println!("   go.mod changed={} restored={}", after != before, o.rolled_back && after == before);
    }
}
