//! Dev harness: `cargo run --example survey -- C:\code [--refresh]` runs a full
//! scan and check against the same SQLite store the app uses.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mehen_core::{CheckOptions, Store, check, scan};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args.iter().find(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| ".".into());
    let options = if args.iter().any(|a| a == "--refresh") { CheckOptions::refresh() } else { CheckOptions::default() };
    let db = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into())).join("Mehen").join("mehen-dev.db");
    let store = Store::open(&db).expect("open store");

    let inventory = scan(std::path::Path::new(&root));
    println!("scan: {} projects in {} ms", inventory.projects.len(), inventory.scan_ms);
    let inventory = check(inventory, &store, options, |_| {}).await;

    let mut by_status: BTreeMap<String, usize> = BTreeMap::new();
    for d in inventory.projects.iter().flat_map(|p| &p.dependencies) {
        *by_status.entry(format!("{:?}", d.status)).or_default() += 1;
    }
    println!("check: {} ms", inventory.check_ms.unwrap_or(0));
    println!("stats: {:?}", inventory.check_stats.unwrap_or_default());
    println!("deps by status: {by_status:?}");
    println!("vulnerabilities: {}", inventory.vulnerabilities.len());
    println!("store: {:?} at {}", store.stats().unwrap(), db.display());
}
