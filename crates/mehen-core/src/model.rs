use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ecosystem {
    Npm,
    Cargo,
    Nuget,
    GithubActions,
    Go,
}

impl Ecosystem {
    /// The name used in settings and the UI (`npm`, `cargo`, `nuget`, `github-actions`, `go`).
    pub fn key(self) -> &'static str {
        match self {
            Ecosystem::Npm => "npm",
            Ecosystem::Cargo => "cargo",
            Ecosystem::Nuget => "nuget",
            Ecosystem::GithubActions => "github-actions",
            Ecosystem::Go => "go",
        }
    }

    /// Ecosystem name as the OSV database spells it.
    pub fn osv_name(self) -> &'static str {
        match self {
            Ecosystem::Npm => "npm",
            Ecosystem::Cargo => "crates.io",
            Ecosystem::Nuget => "NuGet",
            Ecosystem::GithubActions => "GitHub Actions",
            Ecosystem::Go => "Go",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DepKind {
    Normal,
    Dev,
    Build,
    Peer,
    Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Not checked yet.
    Pending,
    /// Workspace, path, git or centrally-managed reference with no version of its own.
    Local,
    /// Branch reference (e.g. `@main`) with nothing to compare.
    Unpinned,
    /// Lookup failed or the version could not be parsed.
    Unknown,
    UpToDate,
    Patch,
    Minor,
    Major,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependency {
    pub name: String,
    pub ecosystem: Ecosystem,
    pub kind: DepKind,
    /// Exactly as written in the manifest: `^18.2.0`, `v4`, a commit SHA.
    pub requested: String,
    /// Exact version from node_modules or a lockfile, when available.
    pub installed: Option<String>,
    /// Where `installed` came from: `node_modules`, `package-lock.json`, `Cargo.lock`...
    pub installed_from: Option<String>,
    /// Version comment next to a SHA-pinned action (`# v4.2.2`).
    pub pinned_comment: Option<String>,
    /// Best-known version currently in use, filled in by the check.
    pub current: Option<String>,
    /// `current` was guessed from a range like `^14.0.0` because nothing was installed or locked.
    pub approximate: bool,
    pub latest: Option<String>,
    /// Newest version on the current release line, when newer than `current`.
    #[serde(default)]
    pub safe_latest: Option<String>,
    /// Newest bug-fix release on the same minor line, when one exists.
    #[serde(default)]
    pub patch_latest: Option<String>,
    /// The newest published version when this project cannot use it (then
    /// `latest` is the newest it can), with the reason.
    #[serde(default)]
    pub newest: Option<String>,
    #[serde(default)]
    pub blocked_reason: Option<String>,
    /// For a vulnerable package: the smallest safe move, the newest release on
    /// the lowest line no advisory covers (15.1.3 when 16.x is also safe).
    #[serde(default)]
    pub fix_target: Option<String>,
    pub status: Status,
    pub vulns: Vec<String>,
    pub note: Option<String>,
}

impl Dependency {
    pub fn new(name: impl Into<String>, ecosystem: Ecosystem, kind: DepKind, requested: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ecosystem,
            kind,
            requested: requested.into(),
            installed: None,
            installed_from: None,
            pinned_comment: None,
            current: None,
            approximate: false,
            latest: None,
            safe_latest: None,
            patch_latest: None,
            newest: None,
            blocked_reason: None,
            fix_target: None,
            status: Status::Pending,
            vulns: Vec::new(),
            note: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// Manifest path; unique per project.
    pub id: String,
    pub name: String,
    pub ecosystem: Ecosystem,
    pub dir: String,
    pub manifest: String,
    pub repo: Option<String>,
    /// Target frameworks for .NET projects.
    pub frameworks: Vec<String>,
    /// Declared minimum Rust version (`rust-version`).
    #[serde(default)]
    pub rust_version: Option<String>,
    /// Exact Node version pinned by `.nvmrc` or `.node-version`.
    #[serde(default)]
    pub node_version: Option<String>,
    /// The `engines.node` range from package.json.
    #[serde(default)]
    pub node_engines: Option<String>,
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vulnerability {
    pub id: String,
    pub aliases: Vec<String>,
    pub summary: String,
    pub severity: Option<String>,
    pub url: String,
    pub fixed: Vec<FixedIn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixedIn {
    pub ecosystem: Ecosystem,
    pub name: String,
    pub versions: Vec<String>,
    /// Which versions the advisory covers. One advisory can hit several
    /// separate lines, like "before 15.1.1" and "16.0.0 up to 16.1.1".
    #[serde(default)]
    pub ranges: Vec<AffectedRange>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AffectedRange {
    /// `None` or `0` for "every version before".
    pub introduced: Option<String>,
    /// First version without the problem.
    pub fixed: Option<String>,
    /// Last version with the problem, when no fix is named.
    pub last_affected: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    /// Watched folders this result covers.
    pub roots: Vec<String>,
    pub projects: Vec<Project>,
    pub vulnerabilities: Vec<Vulnerability>,
    pub skipped_worktrees: Vec<String>,
    /// Folders and manifests skipped because of an ignore rule.
    #[serde(default)]
    pub ignored: Vec<String>,
    pub warnings: Vec<String>,
    pub scan_ms: u64,
    pub check_ms: Option<u64>,
    /// Unix seconds when the check finished.
    #[serde(default)]
    pub checked_at: Option<u64>,
    #[serde(default)]
    pub check_stats: Option<CheckStats>,
}

/// Lowercase with backslashes, so Windows paths compare reliably.
fn norm(path: &str) -> String {
    path.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// True when `path` is `folder` or inside it.
pub fn path_within(path: &str, folder: &str) -> bool {
    let (p, f) = (norm(path), norm(folder));
    p == f || p.starts_with(&format!("{f}\\"))
}

impl Inventory {
    /// Folds in a check of only `scanned` folders: their projects are replaced,
    /// everything else stays as it was, and advisories no project uses anymore
    /// are dropped.
    pub fn merge_partial(mut self, partial: Inventory, scanned: &[String]) -> Inventory {
        let inside = |dir: &str| scanned.iter().any(|s| path_within(dir, s));
        self.projects.retain(|p| !inside(&p.dir));
        self.projects.extend(partial.projects);
        self.projects.sort_by_key(|p| p.dir.to_lowercase());
        let mut seen = std::collections::HashSet::new();
        let used: std::collections::HashSet<String> = self.projects.iter().flat_map(|p| &p.dependencies).flat_map(|d| d.vulns.iter().cloned()).collect();
        self.vulnerabilities = partial.vulnerabilities.into_iter().chain(self.vulnerabilities).filter(|v| used.contains(&v.id) && seen.insert(v.id.clone())).collect();
        self.warnings.retain(|w| !scanned.iter().any(|s| norm(w).contains(&norm(s))));
        self.warnings.extend(partial.warnings);
        self.checked_at = partial.checked_at.or(self.checked_at);
        self.check_stats = partial.check_stats.or(self.check_stats);
        self
    }
}

/// How much of a check came from the local store versus the network.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckStats {
    pub packages_cached: usize,
    pub packages_fetched: usize,
    pub packages_failed: usize,
    pub osv_cached: usize,
    pub osv_fetched: usize,
    pub advisories_cached: usize,
    pub advisories_fetched: usize,
    /// Registries that were still rate limiting after retries.
    pub throttled: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: String,
    pub done: usize,
    pub total: usize,
}

/// A project found by a no-network walk, for choosing what to check.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredProject {
    pub id: String,
    pub name: String,
    pub ecosystem: Ecosystem,
    pub dir: String,
    pub manifest: String,
    pub repo: Option<String>,
    pub dependency_count: usize,
    /// The ignore rule hiding this project, if any.
    pub ignored_by: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(dir: &str, vulns: &[&str]) -> Project {
        let mut dep = Dependency::new("pkg", Ecosystem::Npm, DepKind::Normal, "1.0.0");
        dep.vulns = vulns.iter().map(|v| v.to_string()).collect();
        Project {
            id: format!("{dir}\\package.json"),
            name: dir.into(),
            ecosystem: Ecosystem::Npm,
            dir: dir.into(),
            manifest: format!("{dir}\\package.json"),
            repo: None,
            frameworks: Vec::new(),
            rust_version: None,
            node_version: None,
            node_engines: None,
            dependencies: vec![dep],
        }
    }

    fn vuln(id: &str) -> Vulnerability {
        Vulnerability { id: id.into(), aliases: Vec::new(), summary: String::new(), severity: None, url: String::new(), fixed: Vec::new() }
    }

    fn inventory(projects: Vec<Project>, vulns: Vec<Vulnerability>, at: u64) -> Inventory {
        Inventory {
            roots: vec!["C:\\code".into()],
            projects,
            vulnerabilities: vulns,
            skipped_worktrees: Vec::new(),
            ignored: Vec::new(),
            warnings: Vec::new(),
            scan_ms: 0,
            check_ms: None,
            checked_at: Some(at),
            check_stats: None,
        }
    }

    #[test]
    fn partial_check_replaces_only_its_folders() {
        let old = inventory(vec![project("C:\\code\\a", &["OLD-A"]), project("C:\\code\\b", &["OLD-B"])], vec![vuln("OLD-A"), vuln("OLD-B")], 1);
        let partial = inventory(vec![project("C:\\Code\\A", &["NEW-A"])], vec![vuln("NEW-A")], 2);
        let merged = old.merge_partial(partial, &["c:/code/a".into()]);
        let dirs: Vec<&str> = merged.projects.iter().map(|p| p.dir.as_str()).collect();
        assert_eq!(dirs, vec!["C:\\Code\\A", "C:\\code\\b"]);
        let ids: Vec<&str> = merged.vulnerabilities.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, vec!["NEW-A", "OLD-B"]);
        assert_eq!(merged.roots, vec!["C:\\code"]);
        assert_eq!(merged.checked_at, Some(2));
    }
}
