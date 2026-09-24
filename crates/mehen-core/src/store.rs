//! Local SQLite store. Every network answer (npm, crates.io, NuGet, GitHub
//! tags, OSV) is cached here so repeat checks only ask for what is stale, and
//! each scan is kept so the app can open on the last results.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::ignore::{IgnoreKind, IgnoreRule};
use crate::model::{Ecosystem, Inventory, Vulnerability};
use crate::registry::PackageInfo;
use crate::version::Version;

/// How long each kind of answer stays fresh.
pub const PACKAGE_TTL: Duration = Duration::from_secs(6 * 3600);
pub const NOT_FOUND_TTL: Duration = Duration::from_secs(24 * 3600);
pub const OSV_TTL: Duration = Duration::from_secs(12 * 3600);
pub const ADVISORY_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
const SCANS_KEPT: i64 = 30;

/// A package kept on one release line, everywhere or in one project.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hold {
    pub id: i64,
    pub ecosystem: Ecosystem,
    pub name: String,
    /// A project folder, or `*` for every project.
    pub scope: String,
    /// The release line to stay on: `5` for 5.x, `5.2` for 5.2.x.
    pub line: String,
}

impl Hold {
    pub fn applies_to(&self, folder: &str) -> bool {
        self.scope == "*" || crate::model::path_within(folder, &self.scope) && crate::model::path_within(&self.scope, folder)
    }

    /// Whether `version` stays on the held line.
    pub fn allows(&self, version: &str) -> bool {
        let (Some(v), Some(line)) = (crate::version::Version::parse(version), crate::version::Version::parse(&self.line)) else { return true };
        (0..line.parts.len()).all(|i| v.part(i) == line.part(i))
    }
}

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

/// What one `prune` removed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Pruned {
    pub packages: usize,
    /// Packages whose old versions were dropped.
    pub trimmed: usize,
    pub osv_queries: usize,
    pub advisories: usize,
    pub icons: usize,
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
        if version < 7 {
            // Holds keep a package on a release line; npm answers cached before
            // peer dependencies were read are dropped so the next check has them.
            conn.execute_batch(
                "CREATE TABLE hold (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    ecosystem TEXT NOT NULL,
                    name TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    line TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    UNIQUE (ecosystem, name, scope)
                );
                DELETE FROM package WHERE ecosystem = 'npm';
                PRAGMA user_version = 7;",
            )?;
        }
        if version < 8 {
            // The version a row was trimmed below, so an unchanged row is not rewritten.
            conn.execute_batch("ALTER TABLE package ADD COLUMN kept_from TEXT; PRAGMA user_version = 8;")?;
        }
        if version < 9 {
            // Lets `prune` hand freed pages back to the disk; takes effect after one VACUUM.
            conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL; VACUUM; PRAGMA user_version = 9;")?;
        }
        if version < 10 {
            // Advisories saved before affected ranges were kept are fetched again.
            conn.execute_batch("DELETE FROM advisory; PRAGMA user_version = 10;")?;
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

    pub fn holds(&self) -> Vec<Hold> {
        let conn = self.conn.lock().unwrap();
        let Ok(mut stmt) = conn.prepare("SELECT id, ecosystem, name, scope, line FROM hold ORDER BY name, scope") else { return Vec::new() };
        stmt.query_map([], |r| {
            let eco: String = r.get(1)?;
            Ok((r.get::<_, i64>(0)?, eco, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?))
        })
        .map(|rows| {
            rows.flatten()
                .filter_map(|(id, eco, name, scope, line)| {
                    let ecosystem = [Ecosystem::Npm, Ecosystem::Cargo, Ecosystem::Nuget, Ecosystem::GithubActions].into_iter().find(|e| eco_key(*e) == eco)?;
                    Some(Hold { id, ecosystem, name, scope, line })
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// Keeps `name` on `line` (`5` or `5.2`) in `scope`: a project folder, or `*` for all.
    pub fn put_hold(&self, ecosystem: Ecosystem, name: &str, scope: &str, line: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO hold (ecosystem, name, scope, line, created_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (ecosystem, name, scope) DO UPDATE SET line = excluded.line",
            params![eco_key(ecosystem), name, scope, line, now()],
        )?;
        Ok(())
    }

    pub fn remove_hold(&self, id: i64) -> anyhow::Result<()> {
        self.conn.lock().unwrap().execute("DELETE FROM hold WHERE id = ?1", params![id])?;
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

    /// Drops what no project needs any more: packages nothing uses (unless
    /// kept on a line), versions older than the oldest one in use, advisory
    /// lookups for versions nobody has, advisories nothing points to, and
    /// logos of folders that are gone. Only call it with a full inventory.
    pub fn prune(&self, inventory: &Inventory) -> anyhow::Result<Pruned> {
        // The oldest version in use per package; `None` when some project's
        // version is unknown or not comparable, so nothing is trimmed.
        let mut lowest: HashMap<(String, String), Option<(Version, String)>> = HashMap::new();
        let mut osv_in_use: HashSet<(String, String, String)> = HashSet::new();
        for dep in inventory.projects.iter().flat_map(|p| &p.dependencies) {
            let key = (eco_key(dep.ecosystem).to_string(), dep.name.clone());
            if let Some(v) = crate::check::osv_version(dep) {
                osv_in_use.insert((key.0.clone(), key.1.clone(), v));
            }
            // Action tags name commits for SHA pins, so they are never trimmed.
            let version = dep.current.as_deref().filter(|_| dep.ecosystem != Ecosystem::GithubActions).and_then(|c| Version::parse(c).map(|v| (v, c.to_string())));
            match lowest.get_mut(&key) {
                None => {
                    lowest.insert(key, version);
                }
                Some(slot) => {
                    *slot = match (slot.take(), version) {
                        (Some(a), Some(b)) => Some(if b.0 < a.0 { b } else { a }),
                        _ => None,
                    }
                }
            }
        }
        let held: HashSet<(String, String)> = self.holds().into_iter().map(|h| (eco_key(h.ecosystem).to_string(), h.name)).collect();
        let repos: HashSet<String> = inventory.projects.iter().map(|p| repo_key(p.repo.as_deref().unwrap_or(&p.dir))).collect();

        let mut pruned = Pruned::default();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let rows: Vec<(String, String, Option<String>)> =
            tx.prepare("SELECT ecosystem, name, kept_from FROM package")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
        for (ecosystem, name, kept_from) in rows {
            let key = (ecosystem, name);
            match lowest.get(&key) {
                None if held.contains(&key) => {}
                None => {
                    tx.execute("DELETE FROM package WHERE ecosystem = ?1 AND name = ?2", params![key.0, key.1])?;
                    pruned.packages += 1;
                }
                Some(Some((min, min_raw))) if kept_from.as_deref() != Some(min_raw.as_str()) => {
                    let (versions_json, requirements_json): (Option<String>, Option<String>) = tx.query_row(
                        "SELECT versions_json, requirements_json FROM package WHERE ecosystem = ?1 AND name = ?2",
                        params![key.0, key.1],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )?;
                    let keep = |v: &str| Version::parse(v).is_none_or(|v| v >= *min);
                    let versions: Option<Vec<String>> = versions_json.as_deref().and_then(|j| serde_json::from_str(j).ok());
                    let requirements: Option<Vec<(String, serde_json::Value)>> = requirements_json.as_deref().and_then(|j| serde_json::from_str(j).ok());
                    let before = versions.as_ref().map_or(0, Vec::len) + requirements.as_ref().map_or(0, Vec::len);
                    let versions = versions.map(|list| list.into_iter().filter(|v| keep(v)).collect::<Vec<_>>());
                    let requirements = requirements.map(|list| list.into_iter().filter(|(v, _)| keep(v)).collect::<Vec<_>>());
                    let after = versions.as_ref().map_or(0, Vec::len) + requirements.as_ref().map_or(0, Vec::len);
                    tx.execute(
                        "UPDATE package SET versions_json = COALESCE(?3, versions_json), requirements_json = COALESCE(?4, requirements_json), kept_from = ?5 WHERE ecosystem = ?1 AND name = ?2",
                        params![
                            key.0,
                            key.1,
                            versions.map(|v| serde_json::to_string(&v).unwrap_or_default()),
                            requirements.map(|r| serde_json::to_string(&r).unwrap_or_default()),
                            min_raw
                        ],
                    )?;
                    if after < before {
                        pruned.trimmed += 1;
                    }
                }
                Some(_) => {}
            }
        }

        let queries: Vec<(String, String, String)> =
            tx.prepare("SELECT ecosystem, name, version FROM osv_query")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
        for q in queries.into_iter().filter(|q| !osv_in_use.contains(q)) {
            tx.execute("DELETE FROM osv_query WHERE ecosystem = ?1 AND name = ?2 AND version = ?3", params![q.0, q.1, q.2])?;
            pruned.osv_queries += 1;
        }
        let referenced: HashSet<String> = tx
            .prepare("SELECT vuln_ids_json FROM osv_query")?
            .query_map([], |r| r.get::<_, String>(0))?
            .filter_map(Result::ok)
            .flat_map(|j| serde_json::from_str::<Vec<String>>(&j).unwrap_or_default())
            .chain(inventory.vulnerabilities.iter().map(|v| v.id.clone()))
            .collect();
        let advisories: Vec<String> = tx.prepare("SELECT id FROM advisory")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        for id in advisories.into_iter().filter(|id| !referenced.contains(id)) {
            tx.execute("DELETE FROM advisory WHERE id = ?1", params![id])?;
            pruned.advisories += 1;
        }

        let icons: Vec<String> = tx.prepare("SELECT repo FROM repo_icon")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        for repo in icons.into_iter().filter(|r| !repos.contains(r)) {
            tx.execute("DELETE FROM repo_icon WHERE repo = ?1", params![repo])?;
            pruned.icons += 1;
        }
        tx.commit()?;
        conn.execute_batch("PRAGMA incremental_vacuum;")?;
        Ok(pruned)
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

    #[test]
    fn holds_round_trip_and_match_their_scope() {
        let store = Store::open_in_memory().unwrap();
        store.put_hold(Ecosystem::Npm, "@mui/material", "*", "5").unwrap();
        store.put_hold(Ecosystem::Npm, "eslint", "C:\\code\\app", "8").unwrap();
        store.put_hold(Ecosystem::Npm, "eslint", "C:\\code\\app", "9").unwrap();
        let holds = store.holds();
        assert_eq!(holds.len(), 2, "the same package and scope is updated, not duplicated");
        let eslint = holds.iter().find(|h| h.name == "eslint").unwrap();
        assert_eq!(eslint.line, "9");
        assert!(eslint.applies_to("c:/code/app/"));
        assert!(!eslint.applies_to("C:\\code\\other"));
        assert!(eslint.allows("9.4.0") && !eslint.allows("10.0.0"));
        store.remove_hold(eslint.id).unwrap();
        assert_eq!(store.holds().len(), 1);
    }

    #[test]
    fn prune_drops_what_no_project_needs() {
        use crate::compat::Requirement;
        use crate::model::{DepKind, Dependency, Project};
        let store = Store::open_in_memory().unwrap();
        let info = |versions: &[&str]| PackageInfo {
            latest: versions.last().map(|v| v.to_string()),
            versions: versions.iter().map(|v| v.to_string()).collect(),
            tags: Vec::new(),
            requirements: versions.iter().map(|v| (v.to_string(), Requirement::Peers { peers: vec![("react".into(), "^18".into())] })).collect(),
        };
        store.put_package(Ecosystem::Npm, "@mui/material", &info(&["5.0.0", "5.1.2", "6.0.0", "7.0.0"]));
        store.put_package(Ecosystem::Npm, "left-pad", &info(&["1.0.0"]));
        store.put_package(Ecosystem::Npm, "kept", &info(&["2.0.0"]));
        store.put_hold(Ecosystem::Npm, "kept", "*", "2").unwrap();
        store.put_osv_hits(&[
            (Ecosystem::Npm, "@mui/material".into(), "5.1.2".into(), vec!["GHSA-old".into()]),
            (Ecosystem::Npm, "@mui/material".into(), "6.0.0".into(), vec!["GHSA-now".into()]),
        ]);
        for id in ["GHSA-old", "GHSA-now"] {
            store.put_advisory(&Vulnerability { id: id.into(), aliases: Vec::new(), summary: String::new(), severity: None, url: String::new(), fixed: Vec::new() });
        }
        store.put_repo_icon("C:/gone", Some("C:/gone/logo.png"));

        let mut mui = Dependency::new("@mui/material", Ecosystem::Npm, DepKind::Normal, "^6.0.0");
        mui.current = Some("6.0.0".into());
        let project = Project {
            id: "C:/app/package.json".into(),
            name: "app".into(),
            ecosystem: Ecosystem::Npm,
            dir: "C:/app".into(),
            manifest: "C:/app/package.json".into(),
            repo: None,
            frameworks: Vec::new(),
            rust_version: None,
            node_version: None,
            node_engines: None,
            dependencies: vec![mui],
        };
        let inventory = Inventory { roots: vec!["C:\\".into()], projects: vec![project], ..Default::default() };

        let pruned = store.prune(&inventory).unwrap();
        assert_eq!(pruned, Pruned { packages: 1, trimmed: 1, osv_queries: 1, advisories: 1, icons: 1 });
        let mui = store.package(Ecosystem::Npm, "@mui/material", PACKAGE_TTL).unwrap().unwrap();
        assert_eq!(mui.versions, ["6.0.0", "7.0.0"]);
        assert_eq!(mui.requirements.len(), 2);
        assert!(store.package(Ecosystem::Npm, "left-pad", PACKAGE_TTL).is_none());
        assert!(store.package(Ecosystem::Npm, "kept", PACKAGE_TTL).is_some(), "a held package stays");
        assert!(store.advisory("GHSA-now").is_some() && store.advisory("GHSA-old").is_none());
        assert_eq!(store.prune(&inventory).unwrap(), Pruned::default(), "a second pass has nothing to do");
    }
}

#[cfg(test)]
mod prune_real {
    use super::*;

    /// `MEHEN_DB=<copy of mehen.db> cargo test -p mehen-core prune_real -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn prune_a_copy_of_a_real_database() {
        let path = std::env::var("MEHEN_DB").expect("MEHEN_DB");
        let store = Store::open(Path::new(&path)).unwrap();
        let inventory = store.last_inventory().unwrap();
        println!("before: {:?}", store.stats().unwrap());
        let started = std::time::Instant::now();
        let first = store.prune(&inventory).unwrap();
        let first_ms = started.elapsed().as_millis();
        let started = std::time::Instant::now();
        let second = store.prune(&inventory).unwrap();
        println!("first: {first:?} in {first_ms} ms; second: {second:?} in {} ms", started.elapsed().as_millis());
        println!("after: {:?}", store.stats().unwrap());
    }
}
