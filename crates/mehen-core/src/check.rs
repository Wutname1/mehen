//! Fills in current version, latest version, update status and vulnerabilities.
//! Every network answer goes through the SQLite store, so only stale entries
//! are fetched.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures::{StreamExt, stream};

use crate::compat::{self, ProjectEnv, Requirement};
use crate::model::{AffectedRange, CheckStats, Dependency, Ecosystem, Inventory, Progress, Project, Status, Vulnerability};
use crate::osv::{self, Query};
use crate::registry::{self, PackageInfo};
use crate::store::{self, Hold, Store};
use crate::version::{Version, compare, from_spec, max_version, safe_target, patch_target};

const LOOKUP_CONCURRENCY: usize = 24;

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

    // Vulnerabilities depend only on installed versions, so they are looked up
    // while the registries answer. Actions pinned to a commit only learn their
    // version from the tag lookup and are asked about afterwards.
    let early_keys: BTreeSet<(Ecosystem, String, String)> =
        all_deps(&mut inventory).filter_map(|dep| osv_version(dep).map(|v| (dep.ecosystem, dep.name.clone(), v))).collect();
    let early = async {
        let found = osv_hits(&http, store, options, early_keys).await?;
        let ids: BTreeSet<String> = found.hits.values().flatten().cloned().collect();
        let (_, advisories_fetched) = osv::details(&http, store, ids.into_iter().collect()).await;
        anyhow::Ok((found, advisories_fetched))
    };

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
    let versions = async {
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
    };
    let ((), early) = futures::join!(versions, early);
    drop(lookups);
    stats.throttled = throttled.into_inner().unwrap().into_iter().map(|e| e.osv_name().to_string()).collect();

    let toolchain = Toolchain::detect().await;
    let holds = store.holds();
    for project in &mut inventory.projects {
        let installed: HashMap<String, String> = project
            .dependencies
            .iter()
            .filter(|d| d.ecosystem == Ecosystem::Npm)
            .filter_map(|d| d.current.clone().map(|c| (d.name.clone(), c)))
            .collect();
        let limits = peer_limits(project, &infos);
        let folder = project.repo.clone().unwrap_or_else(|| project.dir.clone());
        let env = ProjectEnv {
            frameworks: project.frameworks.clone(),
            rust: project.rust_version.clone().or_else(|| toolchain.rust.clone()),
            node: project_node(project.node_version.as_deref(), project.node_engines.as_deref(), toolchain.node.as_deref()),
            installed,
            python: project.python_version.clone(),
            dart: toolchain.dart.clone(),
            php: project.php_version.clone().or_else(|| toolchain.php.clone()),
            ruby: toolchain.ruby.clone(),
        };
        for dep in &mut project.dependencies {
            let mut own: Vec<Limit> = limits.get(&dep.name).cloned().unwrap_or_default();
            own.extend(
                holds
                    .iter()
                    .filter(|h| h.ecosystem == dep.ecosystem && h.name == dep.name && h.applies_to(&folder))
                    .map(|h| Limit::Hold(h.clone())),
            );
            apply_info(dep, infos.get(&(dep.ecosystem, dep.name.clone())), &env, &own);
        }
    }

    progress(Progress { phase: "Checking for vulnerabilities".into(), done: 0, total: 1 });
    let early = match early {
        Ok((found, advisories_fetched)) => {
            stats.osv_cached += found.cached;
            stats.osv_fetched += found.fetched;
            stats.advisories_fetched += advisories_fetched;
            found.hits
        }
        Err(_) => HashMap::new(),
    };
    match find_vulnerabilities(&http, store, options, &mut inventory, &mut stats, early).await {
        Ok(vulns) => {
            set_fix_targets(&mut inventory, &infos, &vulns);
            inventory.vulnerabilities = vulns;
        }
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

/// The Node version a project actually runs on: an exact `.nvmrc` pin, else
/// the installed Node when `engines` allows it, else the bottom of the
/// `engines` range. `engines: ">=22"` is a floor, not a pin to 22.0.0.
fn project_node(pinned: Option<&str>, engines: Option<&str>, installed: Option<&str>) -> Option<String> {
    if let Some(p) = pinned {
        return Some(p.to_string());
    }
    match (engines, installed) {
        (Some(range), Some(have)) if compat::node_satisfies(have, range) => Some(have.to_string()),
        (Some(range), _) => from_spec(range),
        (None, have) => have.map(str::to_string),
    }
}

/// Installed Rust and Node, used when a project does not declare its own.
#[derive(Clone)]
struct Toolchain {
    rust: Option<String>,
    node: Option<String>,
    dart: Option<String>,
    php: Option<String>,
    ruby: Option<String>,
}

/// Installed toolchains rarely change, and asking costs two process starts.
const TOOLCHAIN_TTL: Duration = Duration::from_secs(10 * 60);
static TOOLCHAIN: Mutex<Option<(Instant, Toolchain)>> = Mutex::new(None);

impl Toolchain {
    async fn detect() -> Self {
        if let Some((at, known)) = TOOLCHAIN.lock().unwrap().as_ref() {
            if at.elapsed() < TOOLCHAIN_TTL {
                return known.clone();
            }
        }
        // `Dart SDK version: 3.5.0 (stable) ...`
        // `PHP 8.3.4 (cli) ...`, `ruby 3.3.0 (2023-12-25 ...)`.
        let (rust, node, dart, php, ruby) =
            futures::join!(version_of("rustc", 1), version_of("node", 0), version_of("dart", 3), version_of("php", 1), version_of("ruby", 1));
        let found = Toolchain { rust, node, dart, php, ruby };
        *TOOLCHAIN.lock().unwrap() = Some((Instant::now(), found.clone()));
        found
    }
}

/// Runs `<program> --version` and takes the given whitespace-separated word:
/// `rustc 1.97.1 (8bab26f4f 2026-07-14)` -> word 1, `v26.3.1` -> word 0.
async fn version_of(program: &str, word: usize) -> Option<String> {
    // Found the way update steps are, so `dart.bat` (from Flutter) works too.
    let mut cmd = tokio::process::Command::new(crate::update::resolve_program(program));
    cmd.arg("--version").kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let out = tokio::time::timeout(Duration::from_secs(5), cmd.output()).await.ok()?.ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // Older Ruby writes its patch level on: `2.7.8p225`.
    let raw = text.split_whitespace().nth(word)?.trim_start_matches('v').split('p').next()?;
    Version::parse(raw).map(|_| raw.to_string())
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
    dep.newest = None;
    dep.blocked_reason = None;
    dep.fix_target = None;
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

/// A limit on a dependency that comes from outside its own package: another
/// installed package's peer range, or the user keeping it on a release line.
#[derive(Debug, Clone)]
enum Limit {
    Peer { owner: String, range: String },
    Hold(Hold),
}

impl Limit {
    fn check(&self, dep: &str, version: &str) -> Result<(), String> {
        match self {
            Limit::Peer { owner, range } if !compat::semver_satisfies(version, range) => Err(format!("{owner} needs {dep} {}", range.trim())),
            Limit::Hold(h) if !h.allows(version) => Err(format!("kept on {}.x", h.line)),
            _ => Ok(()),
        }
    }
}

/// For each npm package, the peer ranges other installed packages put on it:
/// `@mui/material 5.1.2` asking for `react ^17 || ^18` limits react.
fn peer_limits(project: &Project, infos: &HashMap<(Ecosystem, String), Result<PackageInfo, String>>) -> HashMap<String, Vec<Limit>> {
    let mut limits: HashMap<String, Vec<Limit>> = HashMap::new();
    for dep in project.dependencies.iter().filter(|d| d.ecosystem == Ecosystem::Npm) {
        let (Some(current), Some(Ok(info))) = (dep.current.as_deref(), infos.get(&(Ecosystem::Npm, dep.name.clone()))) else { continue };
        for (version, requirement) in &info.requirements {
            if version != current {
                continue;
            }
            if let Requirement::Peers { peers } = requirement {
                for (name, range) in peers {
                    limits.entry(name.clone()).or_default().push(Limit::Peer { owner: format!("{} {current}", dep.name), range: range.clone() });
                }
            }
        }
    }
    limits
}

fn apply_info(dep: &mut Dependency, info: Option<&Result<PackageInfo, String>>, env: &ProjectEnv, limits: &[Limit]) {
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
    // Only versions this project can actually use count as update targets.
    // When the newest release is out of reach, keep it (and why) for display.
    let mut requirements: HashMap<&str, Vec<&compat::Requirement>> = HashMap::new();
    for (v, r) in &info.requirements {
        requirements.entry(v.as_str()).or_default().push(r);
    }
    let name = dep.name.clone();
    let check = |v: &str| -> Result<(), String> {
        for r in requirements.get(v).into_iter().flatten() {
            compat::check(r, env)?;
        }
        limits.iter().try_for_each(|l| l.check(&name, v))
    };
    // If the version already in use fails the check, our picture of the
    // project's environment is wrong; trust reality and skip filtering rather
    // than suggest a downgrade.
    let env_is_wrong = dep.current.as_deref().is_some_and(|c| {
        let c = c.trim_start_matches(['v', 'V']);
        info.versions.iter().any(|v| v.trim_start_matches(['v', 'V']) == c) && check(c).is_err()
    });
    let usable = |v: &str| if env_is_wrong { Ok(()) } else { check(v) };
    // Older releases are never a target, and checking thousands of them is
    // most of a warm check's time.
    let floor = dep.current.as_deref().and_then(Version::parse);
    let usable_versions: Vec<String> = info
        .versions
        .iter()
        .filter(|v| floor.as_ref().is_none_or(|f| Version::parse(v).is_none_or(|v| v >= *f)))
        .filter(|v| usable(v).is_ok())
        .cloned()
        .collect();
    dep.latest = info.latest.clone();
    if let Some(newest) = &info.latest {
        if let Err(reason) = usable(newest) {
            dep.newest = Some(newest.clone());
            dep.blocked_reason = Some(reason);
            dep.latest = max_version(usable_versions.iter().map(String::as_str));
        }
    }

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
        dep.safe_latest = dep.current.as_deref().and_then(|c| safe_target(c, &usable_versions)).filter(|s| Some(s) != dep.latest.as_ref());
        dep.patch_latest = dep.current.as_deref().and_then(|c| patch_target(c, &usable_versions));
    }
}

async fn find_vulnerabilities(
    http: &reqwest::Client,
    store: &Store,
    options: CheckOptions,
    inventory: &mut Inventory,
    stats: &mut CheckStats,
    mut hits: HashMap<(Ecosystem, String, String), Vec<String>>,
) -> anyhow::Result<Vec<Vulnerability>> {
    let late: BTreeSet<(Ecosystem, String, String)> = all_deps(inventory)
        .filter_map(|dep| osv_version(dep).map(|v| (dep.ecosystem, dep.name.clone(), v)))
        .filter(|key| !hits.contains_key(key))
        .collect();
    let found = osv_hits(http, store, options, late).await?;
    stats.osv_cached += found.cached;
    stats.osv_fetched += found.fetched;
    hits.extend(found.hits);

    let mut ids: BTreeSet<String> = BTreeSet::new();
    for dep in all_deps(inventory) {
        if let Some(found) = osv_version(dep).and_then(|v| hits.get(&(dep.ecosystem, dep.name.clone(), v))) {
            dep.vulns = found.clone();
            ids.extend(found.iter().cloned());
        }
    }

    let (vulns, fetched) = osv::details(http, store, ids.into_iter().collect()).await;
    stats.advisories_fetched += fetched;
    stats.advisories_cached = vulns.len().saturating_sub(stats.advisories_fetched);
    let mut vulns = merge_aliases(vulns, inventory);
    vulns.sort_by(|a, b| severity_rank(&b.severity).cmp(&severity_rank(&a.severity)).then(a.id.cmp(&b.id)));
    Ok(vulns)
}

/// One vulnerability can come back as several records that alias each other
/// (GHSA-... and GO-... for the same Go bug). They become one, keeping the
/// GitHub record since it carries the severity, and each dependency's list
/// is renamed to match.
fn merge_aliases(vulns: Vec<Vulnerability>, inventory: &mut Inventory) -> Vec<Vulnerability> {
    let ids: HashSet<&str> = vulns.iter().map(|v| v.id.as_str()).collect();
    // Union-find over ids that name each other.
    let mut parent: HashMap<String, String> = vulns.iter().map(|v| (v.id.clone(), v.id.clone())).collect();
    fn root(parent: &mut HashMap<String, String>, id: &str) -> String {
        let next = parent[id].clone();
        if next == id {
            return next;
        }
        let top = root(parent, &next);
        parent.insert(id.to_string(), top.clone());
        top
    }
    let rank = |id: &str| (!id.starts_with("GHSA-"), id.to_string());
    for v in &vulns {
        for alias in v.aliases.iter().filter(|a| ids.contains(a.as_str())) {
            let (a, b) = (root(&mut parent, &v.id), root(&mut parent, alias));
            if a != b {
                let (keep, drop) = if rank(&a) <= rank(&b) { (a, b) } else { (b, a) };
                parent.insert(drop, keep);
            }
        }
    }
    let canonical: HashMap<String, String> = vulns.iter().map(|v| (v.id.clone(), root(&mut parent, &v.id))).collect();
    if canonical.iter().all(|(id, top)| id == top) {
        return vulns;
    }

    let mut merged: Vec<Vulnerability> = Vec::new();
    let mut extra: Vec<Vulnerability> = Vec::new();
    for v in vulns {
        if canonical[&v.id] == v.id { merged.push(v) } else { extra.push(v) }
    }
    for other in extra {
        let Some(keep) = merged.iter_mut().find(|m| m.id == canonical[&other.id]) else { continue };
        if keep.severity.is_none() {
            keep.severity = other.severity;
        }
        if !keep.aliases.contains(&other.id) {
            keep.aliases.push(other.id);
        }
        for alias in other.aliases {
            if alias != keep.id && !keep.aliases.contains(&alias) {
                keep.aliases.push(alias);
            }
        }
        for fixed in other.fixed {
            if !keep.fixed.iter().any(|f| f.ecosystem == fixed.ecosystem && f.name == fixed.name) {
                keep.fixed.push(fixed);
            }
        }
    }
    for dep in all_deps(inventory).filter(|d| !d.vulns.is_empty()) {
        let mut renamed: Vec<String> = Vec::new();
        for id in &dep.vulns {
            let top = canonical.get(id).cloned().unwrap_or_else(|| id.clone());
            if !renamed.contains(&top) {
                renamed.push(top);
            }
        }
        dep.vulns = renamed;
    }
    merged
}

fn set_fix_targets(inventory: &mut Inventory, infos: &HashMap<(Ecosystem, String), Result<PackageInfo, String>>, vulns: &[Vulnerability]) {
    let by_id: HashMap<&str, &Vulnerability> = vulns.iter().map(|v| (v.id.as_str(), v)).collect();
    for dep in all_deps(inventory).filter(|d| !d.vulns.is_empty()) {
        let Some(Ok(info)) = infos.get(&(dep.ecosystem, dep.name.clone())) else { continue };
        let ranges: Vec<&AffectedRange> = dep
            .vulns
            .iter()
            .filter_map(|id| by_id.get(id.as_str()))
            .flat_map(|v| v.fixed.iter())
            .filter(|f| f.ecosystem == dep.ecosystem && f.name.eq_ignore_ascii_case(&dep.name))
            .flat_map(|f| f.ranges.iter())
            .collect();
        dep.fix_target = fix_target(dep.current.as_deref(), dep.latest.as_deref(), &info.versions, &ranges);
    }
}

/// The newest release on the lowest line above `current` that none of the
/// ranges cover, no newer than `latest` (the newest the project can use).
fn fix_target(current: Option<&str>, latest: Option<&str>, versions: &[String], ranges: &[&AffectedRange]) -> Option<String> {
    if ranges.is_empty() {
        return None;
    }
    let current = Version::parse(current?)?;
    let cap = latest.and_then(Version::parse);
    // Where minor releases break (0.x), the minor is the line.
    let line = |v: &Version| if v.part(0) == 0 { (0, v.part(1)) } else { (v.part(0), 0) };
    let safe: Vec<(Version, &String)> = versions
        .iter()
        .filter_map(|s| Version::parse(s).map(|v| (v, s)))
        .filter(|(v, _)| *v > current && (current.prerelease || !v.prerelease))
        .filter(|(v, _)| cap.as_ref().is_none_or(|c| v <= c))
        // A range with no end has no fix yet; no update escapes it, so it cannot decide which one to pick.
        .filter(|(v, _)| !ranges.iter().filter(|r| r.fixed.is_some() || r.last_affected.is_some()).any(|r| covers(r, v)))
        .collect();
    let lowest = safe.iter().map(|(v, _)| line(v)).min()?;
    safe.into_iter().filter(|(v, _)| line(v) == lowest).max_by(|a, b| a.0.cmp(&b.0)).map(|(_, s)| s.clone())
}

/// An advisory bound in its ecosystem's own spelling. PyPI writes
/// pre-releases as `5.0a1` or `2.1rc1`: those read as the release they lead
/// up to, marked pre-release so they sort just before it (`1.0.post1` sorts
/// with its release).
fn bound(s: &str) -> Option<Version> {
    Version::parse(s).or_else(|| {
        let release: String = s.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        let release = release.trim_end_matches('.');
        let mut v = Version::parse(release)?;
        v.prerelease = !s[release.len()..].trim_start_matches('.').starts_with("post");
        Some(v)
    })
}

/// Whether an advisory range includes `v`. An unreadable bound counts as
/// covered, so a doubtful version is never offered as the fix.
fn covers(range: &AffectedRange, v: &Version) -> bool {
    let started = range.introduced.as_deref().is_none_or(|i| bound(i).is_none_or(|i| *v >= i));
    let not_ended = match (range.fixed.as_deref(), range.last_affected.as_deref()) {
        (Some(fixed), _) => bound(fixed).is_none_or(|f| *v < f),
        (None, Some(last)) => bound(last).is_none_or(|l| *v <= l),
        (None, None) => true,
    };
    started && not_ended
}

struct OsvFound {
    hits: HashMap<(Ecosystem, String, String), Vec<String>>,
    cached: usize,
    fetched: usize,
}

/// Advisory ids for each package version, from the store when fresh.
async fn osv_hits(http: &reqwest::Client, store: &Store, options: CheckOptions, keys: BTreeSet<(Ecosystem, String, String)>) -> anyhow::Result<OsvFound> {
    let mut found = OsvFound { hits: HashMap::new(), cached: 0, fetched: 0 };
    let mut batch: Vec<Query> = Vec::new();
    for (ecosystem, name, version) in keys {
        match store.osv_hits(ecosystem, &name, &version, options.osv_max_age) {
            Some(ids) => {
                found.cached += 1;
                found.hits.insert((ecosystem, name, version), ids);
            }
            None => batch.push(Query { ecosystem, name, version }),
        }
    }
    if !batch.is_empty() {
        let results = osv::query_batch(http, &batch).await?;
        found.fetched = batch.len();
        let fresh: Vec<(Ecosystem, String, String, Vec<String>)> =
            batch.into_iter().zip(results).map(|(q, ids)| (q.ecosystem, q.name, q.version, ids)).collect();
        store.put_osv_hits(&fresh);
        for (ecosystem, name, version, ids) in fresh {
            found.hits.insert((ecosystem, name, version), ids);
        }
    }
    Ok(found)
}

/// Only exact-enough versions are worth asking about; `v4` or `1.x` would
/// either miss everything or match everything.
pub(crate) fn osv_version(dep: &Dependency) -> Option<String> {
    let current = dep.current.as_deref()?;
    let parsed = Version::parse(current)?;
    (parsed.parts.len() >= 3 || dep.ecosystem == Ecosystem::Nuget).then(|| current.trim_start_matches(['v', 'V']).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compat::Requirement;
    use crate::model::DepKind;

    fn info(versions: &[(&str, Option<&str>)]) -> PackageInfo {
        PackageInfo {
            latest: versions.last().map(|(v, _)| v.to_string()),
            versions: versions.iter().map(|(v, _)| v.to_string()).collect(),
            tags: Vec::new(),
            requirements: versions.iter().filter_map(|(v, fw)| fw.map(|f| (v.to_string(), Requirement::Frameworks { frameworks: vec![f.to_string()] }))).collect(),
        }
    }

    #[test]
    fn aliased_records_become_one_vulnerability() {
        let vuln = |id: &str, aliases: &[&str], severity: Option<&str>| Vulnerability {
            id: id.into(),
            aliases: aliases.iter().map(|a| a.to_string()).collect(),
            summary: String::new(),
            severity: severity.map(Into::into),
            url: String::new(),
            fixed: Vec::new(),
        };
        let mut dep = Dependency::new("golang.org/x/crypto", Ecosystem::Go, DepKind::Normal, "v0.51.0");
        dep.vulns = vec!["GO-2026-5014".into(), "GHSA-45gg".into(), "GO-2026-5099".into()];
        let project = Project {
            id: "p".into(),
            name: "p".into(),
            ecosystem: Ecosystem::Go,
            dir: "C:/app".into(),
            manifest: "C:/app/go.mod".into(),
            repo: None,
            frameworks: Vec::new(),
            rust_version: None,
            node_version: None,
            node_engines: None,
            python_version: None,
            php_version: None,
            dependencies: vec![dep],
        };
        let mut inventory = Inventory { projects: vec![project], ..Default::default() };
        let vulns = vec![vuln("GO-2026-5014", &["GHSA-45gg", "CVE-2026-1"], None), vuln("GHSA-45gg", &["GO-2026-5014", "CVE-2026-1"], Some("HIGH")), vuln("GO-2026-5099", &[], None)];
        let merged = merge_aliases(vulns, &mut inventory);
        let ids: Vec<&str> = merged.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["GHSA-45gg", "GO-2026-5099"]);
        assert_eq!(merged[0].severity.as_deref(), Some("HIGH"));
        assert!(merged[0].aliases.contains(&"GO-2026-5014".to_string()));
        assert_eq!(inventory.projects[0].dependencies[0].vulns, ["GHSA-45gg", "GO-2026-5099"]);
    }

    #[test]
    fn smallest_fix_is_the_newest_release_on_the_lowest_safe_line() {
        let range = |introduced: Option<&str>, fixed: &str| AffectedRange { introduced: introduced.map(Into::into), fixed: Some(fixed.into()), last_affected: None };
        let ranges = [range(None, "15.1.1"), range(Some("16.0.0"), "16.1.1")];
        let ranges: Vec<&AffectedRange> = ranges.iter().collect();
        let versions: Vec<String> = ["12.0.1", "14.0.0", "15.0.0", "15.1.0", "15.1.1", "15.1.3", "16.0.0", "16.1.0", "16.1.1", "16.2.0"].iter().map(|v| v.to_string()).collect();
        assert_eq!(fix_target(Some("12.0.1"), Some("16.2.0"), &versions, &ranges).as_deref(), Some("15.1.3"));
        assert_eq!(fix_target(Some("16.0.0"), Some("16.2.0"), &versions, &ranges).as_deref(), Some("16.2.0"), "on 16.x the fix stays on 16.x");
        assert_eq!(fix_target(Some("12.0.1"), Some("15.1.0"), &versions, &ranges), None, "nothing safe the project can use");
        assert_eq!(fix_target(Some("12.0.1"), Some("16.2.0"), &versions, &[]), None, "no range data, no guess");
        let unfixed = AffectedRange::default();
        let with_unfixed: Vec<&AffectedRange> = ranges.iter().copied().chain([&unfixed]).collect();
        assert_eq!(fix_target(Some("12.0.1"), Some("16.2.0"), &versions, &with_unfixed).as_deref(), Some("15.1.3"), "an advisory nothing fixes is set aside");
        let pre = AffectedRange { introduced: Some("16.0a1".into()), fixed: Some("16.1.1".into()), last_affected: None };
        let v = |s: &str| Version::parse(s).unwrap();
        assert!(!covers(&pre, &v("15.1.3")), "a pre-release start is not every version");
        assert!(covers(&pre, &v("16.0.0")) && !covers(&pre, &v("16.1.1")));
    }

    #[test]
    fn newest_out_of_reach_falls_back_to_newest_usable() {
        let mut dep = Dependency::new("Microsoft.EntityFrameworkCore", Ecosystem::Nuget, DepKind::Normal, "8.0.0");
        reset(&mut dep);
        let env = ProjectEnv { frameworks: vec!["net8.0".into()], ..Default::default() };
        apply_info(&mut dep, Some(&Ok(info(&[("8.0.0", Some("net8.0")), ("9.0.20", Some("net8.0")), ("10.0.12", Some("net10.0"))]))), &env, &[]);
        assert_eq!(dep.latest.as_deref(), Some("9.0.20"));
        assert_eq!(dep.newest.as_deref(), Some("10.0.12"));
        assert_eq!(dep.blocked_reason.as_deref(), Some("only supports net10.0"));
        assert_eq!(dep.status, Status::Major);
    }

    #[test]
    fn never_suggests_a_downgrade_when_the_environment_looks_wrong() {
        let mut dep = Dependency::new("x", Ecosystem::Nuget, DepKind::Normal, "10.0.0");
        reset(&mut dep);
        let env = ProjectEnv { frameworks: vec!["net8.0".into()], ..Default::default() };
        apply_info(&mut dep, Some(&Ok(info(&[("9.0.0", Some("net8.0")), ("10.0.0", Some("net10.0")), ("10.0.5", Some("net10.0"))]))), &env, &[]);
        assert_eq!(dep.latest.as_deref(), Some("10.0.5"));
        assert!(dep.newest.is_none());
    }

    #[test]
    fn node_floor_is_not_a_pin() {
        assert_eq!(project_node(None, Some(">=22"), Some("26.3.1")).as_deref(), Some("26.3.1"));
        assert_eq!(project_node(None, Some("^18"), Some("26.3.1")).as_deref(), Some("18"));
        assert_eq!(project_node(Some("20.11.0"), Some(">=18"), Some("26.3.1")).as_deref(), Some("20.11.0"));
        assert_eq!(project_node(None, None, Some("26.3.1")).as_deref(), Some("26.3.1"));
    }

    fn npm_info(versions: &[&str], peers: &[(&str, &[(&str, &str)])]) -> PackageInfo {
        PackageInfo {
            latest: versions.last().map(|v| v.to_string()),
            versions: versions.iter().map(|v| v.to_string()).collect(),
            tags: Vec::new(),
            requirements: peers
                .iter()
                .map(|(v, list)| (v.to_string(), Requirement::Peers { peers: list.iter().map(|(n, r)| (n.to_string(), r.to_string())).collect() }))
                .collect(),
        }
    }

    fn npm_dep(name: &str, version: &str) -> Dependency {
        let mut dep = Dependency::new(name, Ecosystem::Npm, DepKind::Normal, version);
        dep.installed = Some(version.into());
        reset(&mut dep);
        dep
    }

    #[test]
    fn skips_a_version_whose_peers_the_project_does_not_have() {
        let mut mui = npm_dep("@mui/material", "5.1.2");
        let env = ProjectEnv { installed: [("react".to_string(), "18.3.1".to_string())].into(), ..Default::default() };
        let info = npm_info(&["5.1.2", "6.4.0", "7.0.0"], &[("6.4.0", &[("react", "^17.0.0 || ^18.0.0 || ^19.0.0")]), ("7.0.0", &[("react", "^19.0.0")])]);
        apply_info(&mut mui, Some(&Ok(info)), &env, &[]);
        assert_eq!(mui.latest.as_deref(), Some("6.4.0"));
        assert_eq!(mui.newest.as_deref(), Some("7.0.0"));
        assert_eq!(mui.blocked_reason.as_deref(), Some("needs react ^19.0.0; this project has 18.3.1"));
    }

    #[test]
    fn an_installed_package_caps_its_peer() {
        let mut react = npm_dep("react", "18.2.0");
        let limits = [Limit::Peer { owner: "@mui/material 5.1.2".into(), range: "^17.0.0 || ^18.0.0".into() }];
        apply_info(&mut react, Some(&Ok(npm_info(&["18.2.0", "18.3.1", "19.1.0"], &[]))), &ProjectEnv::default(), &limits);
        assert_eq!(react.latest.as_deref(), Some("18.3.1"));
        assert_eq!(react.newest.as_deref(), Some("19.1.0"));
        assert_eq!(react.blocked_reason.as_deref(), Some("@mui/material 5.1.2 needs react ^17.0.0 || ^18.0.0"));
    }

    #[test]
    fn a_hold_keeps_a_package_on_its_line() {
        let mut mui = npm_dep("@mui/material", "5.1.2");
        let hold = Hold { id: 1, ecosystem: Ecosystem::Npm, name: "@mui/material".into(), scope: "*".into(), line: "5".into() };
        apply_info(&mut mui, Some(&Ok(npm_info(&["5.1.2", "5.16.7", "6.4.0", "7.0.0"], &[]))), &ProjectEnv::default(), &[Limit::Hold(hold)]);
        assert_eq!(mui.latest.as_deref(), Some("5.16.7"));
        assert_eq!(mui.newest.as_deref(), Some("7.0.0"));
        assert_eq!(mui.blocked_reason.as_deref(), Some("kept on 5.x"));
        assert_eq!(mui.status, Status::Minor);
    }

    #[test]
    fn peer_limits_come_from_installed_versions_only() {
        let project = Project {
            id: "p".into(),
            name: "p".into(),
            ecosystem: Ecosystem::Npm,
            dir: "C:\\app".into(),
            manifest: "C:\\app\\package.json".into(),
            repo: None,
            frameworks: Vec::new(),
            rust_version: None,
            node_version: None,
            node_engines: None,
            python_version: None,
            php_version: None,
            dependencies: vec![npm_dep("@mui/material", "5.1.2"), npm_dep("react", "18.2.0")],
        };
        let mut infos = HashMap::new();
        infos.insert(
            (Ecosystem::Npm, "@mui/material".to_string()),
            Ok(npm_info(&["5.1.2", "7.0.0"], &[("5.1.2", &[("react", "^17.0.0 || ^18.0.0")]), ("7.0.0", &[("react", "^19.0.0")])])),
        );
        let limits = peer_limits(&project, &infos);
        let react = &limits["react"];
        assert_eq!(react.len(), 1, "only the installed 5.1.2 limits react, not 7.0.0");
        assert!(react[0].check("react", "19.0.0").is_err());
        assert!(react[0].check("react", "18.3.1").is_ok());
    }
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
