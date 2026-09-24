//! Dev harness: times a warm scan and check the way the app runs them, using
//! the folders and exclusions saved in a store.
//! `cargo run --release --example profile -- <copy of mehen.db> [runs] [--refresh]`

use std::path::PathBuf;
use std::time::Instant;

use mehen_core::{CheckOptions, IgnoreSet, Store, check, scan};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let store = Store::open(&PathBuf::from(&args[0])).expect("open store");
    let runs: usize = args.get(1).and_then(|r| r.parse().ok()).unwrap_or(3);
    let roots: Vec<PathBuf> = store.folders().into_iter().map(PathBuf::from).collect();
    let ignore = IgnoreSet::new(&store.ignore_rules(), &roots);

    for run in 1..=runs {
        let started = Instant::now();
        let inventory = scan(&roots, &ignore);
        let scanned = started.elapsed();
        let last = std::sync::Mutex::new(Instant::now());
        let inventory = check(inventory, &store, if args.iter().any(|a| a == "--refresh") { CheckOptions::refresh() } else { CheckOptions::default() }, |p| {
            let mut last = last.lock().unwrap();
            if p.done == 0 {
                println!("    {:>6} ms  -> {}", last.elapsed().as_millis(), p.phase);
                *last = Instant::now();
            }
        })
        .await;
        // As the app does after a full check.
        store.prune(&inventory).expect("prune");
        let mut ids: Vec<String> = inventory.projects.iter().map(|p| format!("{}:{}", p.id.to_lowercase(), p.dependencies.len())).collect();
        ids.sort();
        let deps: usize = inventory.projects.iter().map(|p| p.dependencies.len()).sum();
        let digest = ids.join("|").bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
        let mut results: Vec<String> = inventory
            .projects
            .iter()
            .flat_map(|p| p.dependencies.iter().map(move |d| format!("{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}", p.id, d.name, d.status, d.latest, d.safe_latest, d.patch_latest, d.newest, d.blocked_reason)))
            .collect();
        results.sort();
        let outcome = results.join("\n").bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
        println!("    results {outcome:x}");
        println!("    fingerprint {digest:x}, {deps} deps, {} vulns, {} ignored", inventory.vulnerabilities.len(), inventory.ignored.len());
        println!(
            "run {run}: {} projects, scan {} ms, check {} ms, total {} ms",
            inventory.projects.len(),
            scanned.as_millis(),
            inventory.check_ms.unwrap_or(0),
            started.elapsed().as_millis()
        );
    }
}
