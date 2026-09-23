//! Dev harness: plans (never applies) moving one package to one version in
//! every project behind it. `cargo run --example bulk_preview -- C:\code actions/checkout v7.0.1`

use std::path::PathBuf;

use mehen_core::update::{Change, plan};
use mehen_core::{CheckOptions, Ecosystem, Status, Store, check, scan};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (root, name, to) = (&args[0], &args[1], &args[2]);
    let db = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap()).join("Mehen").join("mehen-dev.db");
    let store = Store::open(&db).unwrap();
    let inventory = check(scan(&[PathBuf::from(root)], &Default::default()), &store, CheckOptions::default(), |_| {}).await;
    for project in &inventory.projects {
        let changes: Vec<Change> = project
            .dependencies
            .iter()
            .filter(|d| &d.name == name && matches!(d.status, Status::Major | Status::Minor | Status::Patch))
            .map(|d| Change { name: d.name.clone(), from: Some(d.requested.clone()), to: to.clone() })
            .collect();
        if changes.is_empty() {
            continue;
        }
        match plan(project, &changes, |n| store.package_any_age(Ecosystem::GithubActions, n)) {
            Ok(p) => {
                let summary: Vec<String> = p.changes.iter().map(|c| format!("{} => {}", &c.written_before[..c.written_before.len().min(12)], c.written_after)).collect();
                println!("OK   {:<42} {} ({} file(s))", project.name, summary.join(", "), p.edits.len());
            }
            Err(e) => println!("FAIL {:<42} {e:#}", project.name),
        }
    }
}
