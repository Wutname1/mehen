//! Dev harness: `cargo run --example survey -- C:\code [--refresh] [--json out.json]`
//! runs a full scan and check against a dev SQLite store. `--json` saves the
//! result for working on the UI in a plain browser.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mehen_core::{CheckOptions, Store, check, scan};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json_out = args.iter().position(|a| a == "--json").map(|i| i + 1);
    let root = args.iter().enumerate().find(|(i, a)| !a.starts_with("--") && Some(*i) != json_out).map(|(_, a)| a.clone()).unwrap_or_else(|| ".".into());
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
    println!("stats: {:?}", inventory.check_stats.clone().unwrap_or_default());
    println!("deps by status: {by_status:?}");
    println!("vulnerabilities: {}", inventory.vulnerabilities.len());
    println!("store: {:?} at {}", store.stats().unwrap(), db.display());
    if let Some(out) = args.iter().position(|a| a == "--json").and_then(|i| args.get(i + 1)) {
        std::fs::write(out, serde_json::to_string(&inventory).unwrap()).expect("write json");
        println!("wrote {out}");
    }
}
