//! Local SQLite store. Every network answer (npm, crates.io, NuGet, GitHub
//! tags, OSV) is cached here so repeat checks only ask for what is stale, and
//! each scan is kept so the app can open on the last results.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::ignore::{IgnoreKind, IgnoreRule};
use crate::model::{Ecosystem, Inventory, Vulnerability};
use crate::registry::PackageInfo;

/// How long each kind of answer stays fresh.
pub const PACKAGE_TTL: Duration = Duration::from_secs(6 * 3600);
pub const NOT_FOUND_TTL: Duration = Duration::from_secs(24 * 3600);
pub const OSV_TTL: Duration = Duration::from_secs(12 * 3600);
pub const ADVISORY_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
const SCANS_KEPT: i64 = 30;

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

/// Lowercase with backslashes and no trailing separator, so one folder is one row.
fn repo_key(path: &str) -> String {
    path.replace('/', "\\").trim_end_matches('\\').to_lowercase()
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
        if version < 2 {
            conn.execute_batch(
                "CREATE TABLE folder (
                    path TEXT PRIMARY KEY,
                    added_at INTEGER NOT NULL
                );
                CREATE TABLE ignore_rule (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    kind TEXT NOT NULL,
                    value TEXT NOT NULL,
                    note TEXT,
                    created_at INTEGER NOT NULL,
                    UNIQUE (kind, value)
                );
                PRAGMA user_version = 2;",
            )?;
        }
        if version < 3 {
            conn.execute_batch("ALTER TABLE package ADD COLUMN versions_json TEXT; PRAGMA user_version = 3;")?;
        }
        if version < 4 {
            conn.execute_batch("ALTER TABLE package ADD COLUMN requirements_json TEXT; PRAGMA user_version = 4;")?;
        }
        if version < 5 {
            conn.execute_batch("CREATE TABLE setting (key TEXT PRIMARY KEY, value TEXT NOT NULL); PRAGMA user_version = 5;")?;
        }
        if version < 6 {
            // Empty icon_path records that the folder was searched and had no logo.
            conn.execute_batch("CREATE TABLE repo_icon (repo TEXT PRIMARY KEY, icon_path TEXT NOT NULL, searched_at INTEGER NOT NULL); PRAGMA user_version = 6;")?;
        }
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// A cached lookup that is still fresh: `Ok` for a found package, `Err`
    /// for a remembered "not found".
    pub fn package(&self, ecosystem: Ecosystem, name: &str, max_age: Duration) -> Option<Result<PackageInfo, String>> {
        let conn = self.conn.lock().unwrap();
        type Row = (Option<String>, Option<String>, Option<String>, i64, Option<String>, Option<String>);
        let row: Option<Row> = conn
            .query_row(
                "SELECT latest, tags_json, error, fetched_at, versions_json, requirements_json FROM package WHERE ecosystem = ?1 AND name = ?2",
                params![eco_key(ecosystem), name],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()
            .ok()
            .flatten();
        let (latest, tags_json, error, fetched_at, versions_json, requirements_json) = row?;
        match error {
            Some(e) => (fetched_at >= fresh_after(NOT_FOUND_TTL)).then_some(Err(e)),
            // Rows saved before version lists or requirements were kept are refetched.
            None if versions_json.is_none() || requirements_json.is_none() => None,
            None => (fetched_at >= fresh_after(max_age)).then(|| {
                let tags = tags_json.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
                let versions = versions_json.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
                let requirements = requirements_json.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
                Ok(PackageInfo { latest, versions, tags, requirements })
            }),
        }
    }

    /// The cached answer regardless of age, for planning edits that need
    /// details such as tag commits.
    pub fn package_any_age(&self, ecosystem: Ecosystem, name: &str) -> Option<PackageInfo> {
        self.package(ecosystem, name, Duration::from_secs(10 * 365 * 24 * 3600)).and_then(Result::ok)
    }

    pub fn put_package(&self, ecosystem: Ecosystem, name: &str, info: &PackageInfo) {
        let tags = (!info.tags.is_empty()).then(|| serde_json::to_string(&info.tags).unwrap_or_default());
        let versions = serde_json::to_string(&info.versions).unwrap_or_else(|_| "[]".into());
        let requirements = serde_json::to_string(&info.requirements).unwrap_or_else(|_| "[]".into());
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO package (ecosystem, name, latest, tags_json, error, fetched_at, versions_json, requirements_json) VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7)",
            params![eco_key(ecosystem), name, info.latest, tags, now(), versions, requirements],
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
        conn.execute("INSERT INTO scan (root, finished_at, inventory_json) VALUES (?1, ?2, ?3)", params![inventory.roots.join(";"), now(), json])?;
        conn.execute("DELETE FROM scan WHERE id NOT IN (SELECT id FROM scan ORDER BY finished_at DESC, id DESC LIMIT ?1)", params![SCANS_KEPT])?;
        Ok(())
    }

    /// Replaces the newest saved scan, for edits like ignoring a project that
    /// should not count as a new scan.
    pub fn replace_last_inventory(&self, inventory: &Inventory) -> anyhow::Result<()> {
        let json = serde_json::to_string(inventory)?;
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute("UPDATE scan SET inventory_json = ?1 WHERE id = (SELECT MAX(id) FROM scan)", params![json])?;
        drop(conn);
        if updated == 0 {
            self.save_inventory(inventory)?;
        }
        Ok(())
    }

    pub fn last_inventory(&self) -> Option<Inventory> {
        let conn = self.conn.lock().unwrap();
        let json: String = conn
            .query_row("SELECT inventory_json FROM scan ORDER BY finished_at DESC, id DESC LIMIT 1", [], |r| r.get(0))
            .optional()
            .ok()
            .flatten()?;
        serde_json::from_str(&json).ok()
    }

    pub fn setting(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT value FROM setting WHERE key = ?1", params![key], |r| r.get(0)).optional().ok().flatten()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT OR REPLACE INTO setting (key, value) VALUES (?1, ?2)", params![key, value])?;
        Ok(())
    }

    pub fn folders(&self) -> Vec<String> {
        let conn = self.conn.lock().unwrap();
        let Ok(mut stmt) = conn.prepare("SELECT path FROM folder ORDER BY added_at, path") else { return Vec::new() };
        stmt.query_map([], |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    pub fn add_folder(&self, path: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT OR IGNORE INTO folder (path, added_at) VALUES (?1, ?2)", params![path, now()])?;
        Ok(())
    }

    pub fn remove_folder(&self, path: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM folder WHERE path = ?1", params![path])?;
        Ok(())
    }

    pub fn ignore_rules(&self) -> Vec<IgnoreRule> {
        let conn = self.conn.lock().unwrap();
        let Ok(mut stmt) = conn.prepare("SELECT id, kind, value, note FROM ignore_rule ORDER BY created_at, id") else { return Vec::new() };
        stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?)))
            .map(|rows| {
                rows.flatten()
                    .filter_map(|(id, kind, value, note)| Some(IgnoreRule { id, kind: IgnoreKind::parse(&kind)?, value, note }))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn add_ignore_rule(&self, kind: IgnoreKind, value: &str, note: Option<&str>) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO ignore_rule (kind, value, note, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![kind.as_str(), value.trim(), note, now()],
        )?;
        Ok(())
    }

    pub fn remove_ignore_rule(&self, id: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM ignore_rule WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Forgets cached lookups so the next check asks every source again.
    /// Saved scans are kept.
    pub fn clear_cache(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("DELETE FROM package; DELETE FROM osv_query; DELETE FROM advisory;")?;
        Ok(())
    }

    /// The remembered logo for a project folder: `Some(Some(path))` when one
    /// was found, `Some(None)` when the folder was searched and had none, and
    /// `None` when it was never searched.
    pub fn repo_icon(&self, repo: &str) -> Option<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT icon_path FROM repo_icon WHERE repo = ?1", params![repo_key(repo)], |r| r.get::<_, String>(0))
            .optional()
            .ok()
            .flatten()
            .map(|p| (!p.is_empty()).then_some(p))
    }

    pub fn put_repo_icon(&self, repo: &str, icon: Option<&str>) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO repo_icon (repo, icon_path, searched_at) VALUES (?1, ?2, ?3)",
            params![repo_key(repo), icon.unwrap_or(""), now()],
        );
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
        let info = PackageInfo { latest: Some("7.0.1".into()), versions: vec!["v7.0.1".into()], tags: vec![("v7.0.1".into(), "abc".into())], requirements: vec![] };
        store.put_package(Ecosystem::GithubActions, "actions/checkout", &info);
        let cached = store.package(Ecosystem::GithubActions, "actions/checkout", PACKAGE_TTL).unwrap().unwrap();
        assert_eq!(cached.latest.as_deref(), Some("7.0.1"));
        assert_eq!(cached.tags.len(), 1);

        store.put_package_missing(Ecosystem::Npm, "no-such-pkg", "not found");
        assert!(store.package(Ecosystem::Npm, "no-such-pkg", PACKAGE_TTL).unwrap().is_err());

        assert_eq!(store.setting("background_hours"), None);
        store.set_setting("background_hours", "12").unwrap();
        assert_eq!(store.setting("background_hours").as_deref(), Some("12"));

        store.add_folder("C:\\code").unwrap();
        store.add_folder("C:\\code").unwrap();
        assert_eq!(store.folders(), vec!["C:\\code".to_string()]);
        store.add_ignore_rule(IgnoreKind::Pattern, "_spikes", None).unwrap();
        let rules = store.ignore_rules();
        assert_eq!(rules.len(), 1);
        store.remove_ignore_rule(rules[0].id).unwrap();
        assert!(store.ignore_rules().is_empty());

        store.put_osv_hits(&[(Ecosystem::Npm, "left-pad".into(), "1.0.0".into(), vec!["GHSA-1".into()])]);
        assert_eq!(store.osv_hits(Ecosystem::Npm, "left-pad", "1.0.0", OSV_TTL), Some(vec!["GHSA-1".to_string()]));
        assert_eq!(store.osv_hits(Ecosystem::Npm, "left-pad", "1.0.1", OSV_TTL), None);
    }
}
