//! Fills in current version, latest version, update status and vulnerabilities.
//! Every network answer goes through the SQLite store, so only stale entries
//! are fetched.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures::{StreamExt, stream};

use crate::model::{CheckStats, Dependency, Ecosystem, Inventory, Progress, Status, Vulnerability};
use crate::osv::{self, Query};
use crate::registry::{self, PackageInfo};
use crate::store::{self, Store};
use crate::version::{Version, compare, from_spec, safe_target};

const LOOKUP_CONCURRENCY: usize = 8;

#[derive(Debug, Clone, Copy)]
pub struct CheckOptions {
    /// Cached latest-version answers younger than this are reused.
    pub package_max_age: Duration,
    /// Cached vulnerability matches younger than this are reused.
    pub osv_max_age: Duration,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self { package_max_age: store::PACKAGE_TTL, osv_max_age: store::OSV_TTL }
    }
}

impl CheckOptions {
    /// Ignores cached answers (except remembered 404s) and asks every source again.
    pub fn refresh() -> Self {
        Self { package_max_age: Duration::ZERO, osv_max_age: Duration::ZERO }
    }
}

pub async fn check(mut inventory: Inventory, store: &Store, options: CheckOptions, progress: impl Fn(Progress)) -> Inventory {
    let start = Instant::now();
    let http = registry::http_client();
    let mut stats = CheckStats::default();

    let mut wanted: BTreeSet<(Ecosystem, String)> = BTreeSet::new();
    for dep in all_deps(&mut inventory) {
        reset(dep);
        if dep.status == Status::Pending {
            wanted.insert((dep.ecosystem, dep.name.clone()));
        }
    }

    let mut infos: HashMap<(Ecosystem, String), Result<PackageInfo, String>> = HashMap::new();
    let mut to_fetch = Vec::new();
    for key in wanted {
        match store.package(key.0, &key.1, options.package_max_age) {
            Some(hit) => {
                stats.packages_cached += 1;
                infos.insert(key, hit);
            }
            None => to_fetch.push(key),
        }
    }

    let total = to_fetch.len();
    progress(Progress { phase: "Checking latest versions".into(), done: 0, total });
    // Once a registry is still answering 429 after retries, stop asking it this
    // run rather than letting every remaining lookup back off in turn.
    let throttled: Mutex<HashSet<Ecosystem>> = Mutex::new(HashSet::new());
    let mut lookups = stream::iter(to_fetch)
        .map(|key| {
            let (http, throttled) = (&http, &throttled);
            async move {
                if throttled.lock().unwrap().contains(&key.0) {
                    return (key, Err("registry is rate limiting requests; try again later".to_string()));
                }
                let result = match registry::lookup(http, key.0, &key.1).await {
                    Ok(info) => {
                        store.put_package(key.0, &key.1, &info);
                        Ok(info)
                    }
                    Err(e) => {
                        let message = e.to_string();
                        if message.contains("404") {
                            store.put_package_missing(key.0, &key.1, "not found in the registry");
                        } else if message.contains("429") {
                            throttled.lock().unwrap().insert(key.0);
                        }
                        Err(message)
                    }
                };
                (key, result)
            }
        })
        .buffer_unordered(LOOKUP_CONCURRENCY);
    let mut done = 0;
    while let Some((key, result)) = lookups.next().await {
        done += 1;
        stats.packages_fetched += 1;
        if result.is_err() {
            stats.packages_failed += 1;
        }
        infos.insert(key, result);
        progress(Progress { phase: "Checking latest versions".into(), done, total });
    }
    drop(lookups);
    stats.throttled = throttled.into_inner().unwrap().into_iter().map(|e| e.osv_name().to_string()).collect();

    for dep in all_deps(&mut inventory) {
        apply_info(dep, infos.get(&(dep.ecosystem, dep.name.clone())));
    }

    progress(Progress { phase: "Checking for vulnerabilities".into(), done: 0, total: 1 });
    match find_vulnerabilities(&http, store, options, &mut inventory, &mut stats).await {
        Ok(vulns) => inventory.vulnerabilities = vulns,
        Err(e) => inventory.warnings.push(format!("Vulnerability check failed: {e}")),
    }
    progress(Progress { phase: "Done".into(), done: 1, total: 1 });

    inventory.check_ms = Some(start.elapsed().as_millis() as u64);
    inventory.check_stats = Some(stats);
    inventory.checked_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs());
    if let Err(e) = store.save_inventory(&inventory) {
        inventory.warnings.push(format!("Could not save scan: {e}"));
    }
    inventory
}

fn all_deps(inventory: &mut Inventory) -> impl Iterator<Item = &mut Dependency> {
    inventory.projects.iter_mut().flat_map(|p| p.dependencies.iter_mut())
}

fn is_commit_sha(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Clears results from a previous check and decides what "current" means before lookup.
fn reset(dep: &mut Dependency) {
    dep.latest = None;
    dep.safe_latest = None;
    dep.vulns.clear();
    dep.approximate = false;
    if dep.status == Status::Local {
        return;
    }
    dep.status = Status::Pending;
    dep.current = match dep.ecosystem {
        Ecosystem::GithubActions if is_commit_sha(&dep.requested) => None,
        Ecosystem::GithubActions if Version::parse(&dep.requested).is_some() => Some(dep.requested.clone()),
        Ecosystem::GithubActions => {
            dep.status = Status::Unpinned;
            None
        }
        Ecosystem::Nuget => dep.installed.clone().or_else(|| from_spec(&dep.requested)),
        _ => dep.installed.clone().or_else(|| from_spec(&dep.requested).inspect(|_| dep.approximate = true)),
    };
}

fn apply_info(dep: &mut Dependency, info: Option<&Result<PackageInfo, String>>) {
    if dep.status != Status::Pending {
        return;
    }
    let info = match info {
        Some(Ok(info)) => info,
        Some(Err(e)) => {
            dep.status = Status::Unknown;
            dep.note = Some(format!("lookup failed: {e}"));
            return;
        }
        None => {
            dep.status = Status::Unknown;
            return;
        }
    };
    dep.latest = info.latest.clone();

    if dep.ecosystem == Ecosystem::GithubActions && is_commit_sha(&dep.requested) {
        dep.current = info.tag_for_commit(&dep.requested).or_else(|| dep.pinned_comment.clone());
    }

    dep.status = match (&dep.current, &dep.latest) {
        (Some(current), Some(latest)) => compare(current, latest),
        _ => Status::Unknown,
    };

    // A floating `v4` action tag already tracks its whole major line.
    let floating_tag = dep.ecosystem == Ecosystem::GithubActions && Version::parse(&dep.requested).is_some_and(|v| v.parts.len() == 1);
    if !floating_tag {
        dep.safe_latest = dep.current.as_deref().and_then(|c| safe_target(c, &info.versions)).filter(|s| Some(s) != dep.latest.as_ref());
    }
}

async fn find_vulnerabilities(
    http: &reqwest::Client,
    store: &Store,
    options: CheckOptions,
    inventory: &mut Inventory,
    stats: &mut CheckStats,
) -> anyhow::Result<Vec<Vulnerability>> {
    // Only exact-enough versions are worth asking about; `v4` or `1.x` would
    // either miss everything or match everything.
    let unique: BTreeSet<(Ecosystem, String, String)> =
        all_deps(inventory).filter_map(|dep| osv_version(dep).map(|v| (dep.ecosystem, dep.name.clone(), v))).collect();

    let mut hits: HashMap<(Ecosystem, String, String), Vec<String>> = HashMap::new();
    let mut batch: Vec<Query> = Vec::new();
    for (ecosystem, name, version) in unique {
        match store.osv_hits(ecosystem, &name, &version, options.osv_max_age) {
            Some(ids) => {
                stats.osv_cached += 1;
                hits.insert((ecosystem, name, version), ids);
            }
            None => batch.push(Query { ecosystem, name, version }),
        }
    }

    if !batch.is_empty() {
        let results = osv::query_batch(http, &batch).await?;
        stats.osv_fetched += batch.len();
        let fresh: Vec<(Ecosystem, String, String, Vec<String>)> =
            batch.into_iter().zip(results).map(|(q, ids)| (q.ecosystem, q.name, q.version, ids)).collect();
        store.put_osv_hits(&fresh);
        for (ecosystem, name, version, ids) in fresh {
            hits.insert((ecosystem, name, version), ids);
        }
    }

    let mut ids: BTreeSet<String> = BTreeSet::new();
    for dep in all_deps(inventory) {
        if let Some(found) = osv_version(dep).and_then(|v| hits.get(&(dep.ecosystem, dep.name.clone(), v))) {
            dep.vulns = found.clone();
            ids.extend(found.iter().cloned());
        }
    }

    let (mut vulns, fetched) = osv::details(http, store, ids.into_iter().collect()).await;
    stats.advisories_fetched = fetched;
    stats.advisories_cached = vulns.len() - fetched;
    vulns.sort_by(|a, b| severity_rank(&b.severity).cmp(&severity_rank(&a.severity)).then(a.id.cmp(&b.id)));
    Ok(vulns)
}

fn osv_version(dep: &Dependency) -> Option<String> {
    let current = dep.current.as_deref()?;
    let parsed = Version::parse(current)?;
    (parsed.parts.len() >= 3 || dep.ecosystem == Ecosystem::Nuget).then(|| current.trim_start_matches(['v', 'V']).to_string())
}

fn severity_rank(severity: &Option<String>) -> u8 {
    match severity.as_deref() {
        Some("CRITICAL") => 4,
        Some("HIGH") => 3,
        Some("MODERATE" | "MEDIUM") => 2,
        Some("LOW") => 1,
        _ => 0,
    }
}
