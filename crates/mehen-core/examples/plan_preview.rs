//! Dev harness: plans (never applies) updates for one real project per
//! ecosystem and prints the diffs. `cargo run --example plan_preview -- C:\code`

use std::path::PathBuf;

use mehen_core::update::{Change, plan};
use mehen_core::{CheckOptions, Ecosystem, Status, Store, check, scan};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let db = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap()).join("Mehen").join("mehen-dev.db");
    let store = Store::open(&db).unwrap();
    let inventory = check(scan(&[PathBuf::from(&root)], &Default::default()), &store, CheckOptions::default(), |_| {}).await;

    for eco in [Ecosystem::Npm, Ecosystem::Cargo, Ecosystem::Nuget, Ecosystem::GithubActions] {
        let candidate = inventory.projects.iter().filter(|p| p.ecosystem == eco).find_map(|p| {
            let changes: Vec<Change> = p
                .dependencies
                .iter()
                .filter(|d| matches!(d.status, Status::Major | Status::Minor | Status::Patch))
                .filter(|d| eco != Ecosystem::GithubActions || d.requested.len() == 40 || d.requested.starts_with('v'))
                .take(2)
                .filter_map(|d| Some(Change { from: None, name: d.name.clone(), to: d.safe_latest.clone().or(d.latest.clone())? }))
                .collect();
            (!changes.is_empty()).then_some((p, changes))
        });
        let Some((project, changes)) = candidate else { continue };
        println!("===== {eco:?}: {} ({})", project.name, project.manifest);
        match plan(project, &changes, |name| store.package_any_age(Ecosystem::GithubActions, name)) {
            Ok(p) => {
                for c in &p.changes {
                    println!("  {}: {} -> {}   [{} => {}]", c.name, c.from, c.to, c.written_before, c.written_after);
                }
                for e in &p.edits {
                    print!("{}", e.diff);
                }
                for s in &p.steps {
                    println!("  step {:?}: {} {} (in {})", s.kind, s.program, s.args.join(" "), s.cwd);
                }
                for w in &p.warnings {
                    println!("  warning: {w}");
                }
            }
            Err(e) => println!("  PLAN FAILED: {e:#}"),
        }
    }
}
