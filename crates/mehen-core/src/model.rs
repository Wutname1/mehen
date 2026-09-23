use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ecosystem {
    Npm,
    Cargo,
    Nuget,
    GithubActions,
}

impl Ecosystem {
    /// Ecosystem name as the OSV database spells it.
    pub fn osv_name(self) -> &'static str {
        match self {
            Ecosystem::Npm => "npm",
            Ecosystem::Cargo => "crates.io",
            Ecosystem::Nuget => "NuGet",
            Ecosystem::GithubActions => "GitHub Actions",
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
