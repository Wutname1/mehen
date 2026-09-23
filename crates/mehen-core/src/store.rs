//! Local SQLite store. Every network answer (npm, crates.io, NuGet, GitHub
//! tags, OSV) is cached here so repeat checks only ask for what is stale, and
//! each scan is kept so the app can open on the last results.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::model::{Ecosystem, Inventory, Vulnerability};
use crate::registry::PackageInfo;

/// How long each kind of answer stays fresh.
pub const PACKAGE_TTL: Duration = Duration::from_secs(6 * 3600);
pub const NOT_FOUND_TTL: Duration = Duration::from_secs(24 * 3600);
pub const OSV_TTL: Duration = Duration::from_secs(12 * 3600);
pub const ADVISORY_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
const SCANS_KEPT_PER_ROOT: i64 = 20;

pub struct Store {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreStats {
    pub packages: i64,
    pub osv_queries: i64,
    pub advisories: i64,
    pub scans: i64,
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn fresh_after(max_age: Duration) -> i64 {
    now() - max_age.as_secs() as i64
}

fn eco_key(ecosystem: Ecosystem) -> &'static str {
    match ecosystem {
        Ecosystem::Npm => "npm",
        Ecosystem::Cargo => "cargo",
        Ecosystem::Nuget => "nuget",
        Ecosystem::GithubActions => "github-actions",
    }
}

impl Store {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> anyhow::Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(
                "CREATE TABLE package (
                    ecosystem TEXT NOT NULL,
                    name TEXT NOT NULL,
                    latest TEXT,
                    tags_json TEXT,
                    error TEXT,
                    fetched_at INTEGER NOT NULL,
                    PRIMARY KEY (ecosystem, name)
                );
                CREATE TABLE osv_query (
                    ecosystem TEXT NOT NULL,
                    name TEXT NOT NULL,
                    version TEXT NOT NULL,
                    vuln_ids_json TEXT NOT NULL,
                    fetched_at INTEGER NOT NULL,
                    PRIMARY KEY (ecosystem, name, version)
                );
                CREATE TABLE advisory (
                    id TEXT PRIMARY KEY,
                    json TEXT NOT NULL,
                    fetched_at INTEGER NOT NULL
                );
                CREATE TABLE scan (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    root TEXT NOT NULL,
                    finished_at INTEGER NOT NULL,
                    inventory_json TEXT NOT NULL
                );
                CREATE INDEX scan_root ON scan (root, finished_at);
                PRAGMA user_version = 1;",
            )?;
        }
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// A cached lookup that is still fresh: `Ok` for a found package, `Err`
    /// for a remembered "not found".
    pub fn package(&self, ecosystem: Ecosystem, name: &str, max_age: Duration) -> Option<Result<PackageInfo, String>> {
        let conn = self.conn.lock().unwrap();
        let row: Option<(Option<String>, Option<String>, Option<String>, i64)> = conn
            .query_row(
                "SELECT latest, tags_json, error, fetched_at FROM package WHERE ecosystem = ?1 AND name = ?2",
                params![eco_key(ecosystem), name],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .ok()
            .flatten();
        let (latest, tags_json, error, fetched_at) = row?;
        match error {
            Some(e) => (fetched_at >= fresh_after(NOT_FOUND_TTL)).then_some(Err(e)),
            None => (fetched_at >= fresh_after(max_age)).then(|| {
                let tags = tags_json.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
                Ok(PackageInfo { latest, tags })
            }),
        }
    }

    pub fn put_package(&self, ecosystem: Ecosystem, name: &str, info: &PackageInfo) {
        let tags = (!info.tags.is_empty()).then(|| serde_json::to_string(&info.tags).unwrap_or_default());
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO package (ecosystem, name, latest, tags_json, error, fetched_at) VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
            params![eco_key(ecosystem), name, info.latest, tags, now()],
        );
    }

    /// Only for answers that will not change on a retry, like a 404.
    pub fn put_package_missing(&self, ecosystem: Ecosystem, name: &str, error: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO package (ecosystem, name, latest, tags_json, error, fetched_at) VALUES (?1, ?2, NULL, NULL, ?3, ?4)",
            params![eco_key(ecosystem), name, error, now()],
        );
    }

    pub fn osv_hits(&self, ecosystem: Ecosystem, name: &str, version: &str, max_age: Duration) -> Option<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let json: String = conn
            .query_row(
                "SELECT vuln_ids_json FROM osv_query WHERE ecosystem = ?1 AND name = ?2 AND version = ?3 AND fetched_at >= ?4",
                params![eco_key(ecosystem), name, version, fresh_after(max_age)],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()?;
        serde_json::from_str(&json).ok()
    }

    pub fn put_osv_hits(&self, entries: &[(Ecosystem, String, String, Vec<String>)]) {
        let mut conn = self.conn.lock().unwrap();
        let Ok(tx) = conn.transaction() else { return };
        let stamp = now();
        for (ecosystem, name, version, ids) in entries {
            let _ = tx.execute(
                "INSERT OR REPLACE INTO osv_query (ecosystem, name, version, vuln_ids_json, fetched_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![eco_key(*ecosystem), name, version, serde_json::to_string(ids).unwrap_or_default(), stamp],
            );
        }
        let _ = tx.commit();
    }

    pub fn advisory(&self, id: &str) -> Option<Vulnerability> {
        let conn = self.conn.lock().unwrap();
        let json: String = conn
            .query_row(
                "SELECT json FROM advisory WHERE id = ?1 AND fetched_at >= ?2",
                params![id, fresh_after(ADVISORY_TTL)],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()?;
        serde_json::from_str(&json).ok()
    }

    pub fn put_advisory(&self, vuln: &Vulnerability) {
        let Ok(json) = serde_json::to_string(vuln) else { return };
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute("INSERT OR REPLACE INTO advisory (id, json, fetched_at) VALUES (?1, ?2, ?3)", params![vuln.id, json, now()]);
    }

    pub fn save_inventory(&self, inventory: &Inventory) -> anyhow::Result<()> {
        let json = serde_json::to_string(inventory)?;
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT INTO scan (root, finished_at, inventory_json) VALUES (?1, ?2, ?3)", params![inventory.root, now(), json])?;
        conn.execute(
            "DELETE FROM scan WHERE root = ?1 AND id NOT IN (SELECT id FROM scan WHERE root = ?1 ORDER BY finished_at DESC, id DESC LIMIT ?2)",
            params![inventory.root, SCANS_KEPT_PER_ROOT],
        )?;
        Ok(())
    }

    pub fn last_inventory(&self, root: &str) -> Option<Inventory> {
        let conn = self.conn.lock().unwrap();
        let json: String = conn
            .query_row("SELECT inventory_json FROM scan WHERE root = ?1 ORDER BY finished_at DESC, id DESC LIMIT 1", params![root], |r| r.get(0))
            .optional()
            .ok()
            .flatten()?;
        serde_json::from_str(&json).ok()
    }

    /// Forgets cached lookups so the next check asks every source again.
    /// Saved scans are kept.
    pub fn clear_cache(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("DELETE FROM package; DELETE FROM osv_query; DELETE FROM advisory;")?;
        Ok(())
    }

    pub fn stats(&self) -> anyhow::Result<StoreStats> {
        let conn = self.conn.lock().unwrap();
        let count = |table: &str| -> rusqlite::Result<i64> { conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)) };
        Ok(StoreStats { packages: count("package")?, osv_queries: count("osv_query")?, advisories: count("advisory")?, scans: count("scan")? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_round_trip_and_expire() {
        let store = Store::open_in_memory().unwrap();
        let info = PackageInfo { latest: Some("7.0.1".into()), tags: vec![("v7.0.1".into(), "abc".into())] };
        store.put_package(Ecosystem::GithubActions, "actions/checkout", &info);
        let cached = store.package(Ecosystem::GithubActions, "actions/checkout", PACKAGE_TTL).unwrap().unwrap();
        assert_eq!(cached.latest.as_deref(), Some("7.0.1"));
        assert_eq!(cached.tags.len(), 1);

        store.put_package_missing(Ecosystem::Npm, "no-such-pkg", "not found");
        assert!(store.package(Ecosystem::Npm, "no-such-pkg", PACKAGE_TTL).unwrap().is_err());

        store.put_osv_hits(&[(Ecosystem::Npm, "left-pad".into(), "1.0.0".into(), vec!["GHSA-1".into()])]);
        assert_eq!(store.osv_hits(Ecosystem::Npm, "left-pad", "1.0.0", OSV_TTL), Some(vec!["GHSA-1".to_string()]));
        assert_eq!(store.osv_hits(Ecosystem::Npm, "left-pad", "1.0.1", OSV_TTL), None);
    }
}
