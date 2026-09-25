//! A small file that tells other apps what the last check found in each git
//! repository, so they can show it without opening Mehen's database. GitWyrm
//! reads it to show a repository's dependency safety.
//!
//! The file is a contract with apps that ship on their own schedule: fields may
//! be added, but never renamed or removed without raising [`FORMAT`]. Readers
//! skip a file whose format they do not know.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{Inventory, Status, path_within};

pub const FILE_NAME: &str = "repo-status.json";
pub const FORMAT: u32 = 1;
/// Problems listed per repository; the counts always cover all of them.
const PROBLEMS_KEPT: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusFile {
    pub format: u32,
    /// The app and version that wrote the file (`Mehen 0.1.0`).
    pub written_by: String,
    /// The program that wrote the file, so a reader can open it again.
    pub exe: Option<String>,
    /// Unix seconds.
    pub written_at: u64,
    /// Unix seconds when every watched folder was last checked. Checks of a
    /// single folder leave it alone, so readers can tell when a full check is due.
    #[serde(default)]
    pub full_check_at: Option<u64>,
    /// `exe` can run a check with no window (`--background-check`). Always
    /// true from this version on; a reader must not start background checks
    /// through a copy of Mehen that did not say so, or an older one would
    /// open its window instead.
    #[serde(default)]
    pub background_check: bool,
    pub repos: Vec<RepoStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoStatus {
    /// The repository's folder as Mehen found it. Compare without case and
    /// with either slash on Windows.
    pub path: String,
    /// Unix seconds when this repository was last checked.
    pub checked_at: Option<u64>,
    /// The commit checked out when it was checked, when it could be read.
    pub checked_commit: Option<String>,
    /// Packages with at least one known vulnerability.
    pub vulnerable: u32,
    /// The vulnerable packages that have a fixed version this repository can
    /// move to. Readers that only want problems someone can act on use this.
    #[serde(default)]
    pub fixable: u32,
    /// Vulnerable packages by their worst advisory.
    pub severity: SeverityCounts,
    /// Packages with a newer version available.
    pub outdated: u32,
    /// Fixable packages first, then by how serious they are.
    pub problems: Vec<Problem>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeverityCounts {
    pub critical: u32,
    pub high: u32,
    pub moderate: u32,
    pub low: u32,
    pub unknown: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub name: String,
    pub ecosystem: String,
    pub version: Option<String>,
    /// The smallest version that fixes every advisory, when one is known.
    pub fixed_in: Option<String>,
    /// `CRITICAL`, `HIGH`, `MODERATE` or `LOW`; absent when the advisory has none.
    pub severity: Option<String>,
    pub summary: String,
    pub advisory: String,
    pub url: String,
}

/// Which repositories the inventory being written has fresh answers for.
pub enum Fresh<'a> {
    /// A full check.
    All,
    /// A check of these folders only; the rest are carried over.
    Only(&'a [String]),
    /// Nothing new was checked (a project was excluded, say).
    None,
}

fn rank(severity: Option<&str>) -> u8 {
    match severity {
        Some("CRITICAL") => 4,
        Some("HIGH") => 3,
        Some("MODERATE" | "MEDIUM") => 2,
        Some("LOW") => 1,
        _ => 0,
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The commit checked out in `repo`, read from `.git` without running git.
/// Handles linked worktrees (`.git` is a file) and packed refs.
pub fn head_commit(repo: &Path) -> Option<String> {
    let dot_git = repo.join(".git");
    let git_dir = if dot_git.is_file() {
        let text = fs::read_to_string(&dot_git).ok()?;
        let target = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
        if target.is_absolute() { target } else { repo.join(target) }
    } else {
        dot_git
    };
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    let Some(reference) = head.strip_prefix("ref:").map(str::trim) else {
        return is_sha(head).then(|| head.to_string());
    };
    // A linked worktree keeps its own HEAD but shares branches with the main repo.
    let common = match fs::read_to_string(git_dir.join("commondir")) {
        Ok(rel) => git_dir.join(rel.trim()),
        Err(_) => git_dir.clone(),
    };
    for dir in [&git_dir, &common] {
        if let Ok(sha) = fs::read_to_string(dir.join(reference)) {
            let sha = sha.trim();
            if is_sha(sha) {
                return Some(sha.to_string());
            }
        }
    }
    let packed = fs::read_to_string(common.join("packed-refs")).ok()?;
    packed.lines().find_map(|line| {
        let (sha, name) = line.split_once(' ')?;
        (name == reference && is_sha(sha)).then(|| sha.to_string())
    })
}

fn is_sha(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// What `inventory` says about each git repository. Projects outside a git
/// repository are left out: nothing that reads this file can open them.
pub fn build(inventory: &Inventory, fresh: Fresh, previous: Option<&StatusFile>, written_by: &str, exe: Option<&str>) -> StatusFile {
    let advisories: HashMap<&str, _> = inventory.vulnerabilities.iter().map(|v| (v.id.as_str(), v)).collect();
    let full_check_at = if matches!(fresh, Fresh::All) { inventory.checked_at } else { previous.and_then(|p| p.full_check_at) };
    let before: HashMap<String, &RepoStatus> = previous.map(|p| p.repos.iter().map(|r| (key(&r.path), r)).collect()).unwrap_or_default();

    // Repository -> package (ecosystem, name) -> the usages found in it.
    let mut by_repo: BTreeMap<String, (String, BTreeMap<(String, String), Vec<&crate::model::Dependency>>)> = BTreeMap::new();
    for project in &inventory.projects {
        let Some(repo) = &project.repo else { continue };
        let entry = by_repo.entry(key(repo)).or_insert_with(|| (repo.clone(), BTreeMap::new()));
        for dep in &project.dependencies {
            entry.1.entry((dep.ecosystem.key().to_string(), dep.name.clone())).or_default().push(dep);
        }
    }

    let repos = by_repo
        .into_values()
        .map(|(path, packages)| {
            let is_fresh = match fresh {
                Fresh::All => true,
                Fresh::Only(folders) => folders.iter().any(|f| path_within(&path, f) || path_within(f, &path)),
                Fresh::None => false,
            };
            let (checked_at, checked_commit) = match before.get(&key(&path)) {
                Some(old) if !is_fresh => (old.checked_at, old.checked_commit.clone()),
                // Carried over with no earlier record: the time is the best
                // guess there is, but the commit would be a claim it cannot make.
                None if !is_fresh => (inventory.checked_at, None),
                _ => (inventory.checked_at, head_commit(Path::new(&path))),
            };

            let mut severity = SeverityCounts::default();
            let mut problems = Vec::new();
            let mut outdated = 0;
            for ((ecosystem, name), usages) in &packages {
                if usages.iter().any(|d| matches!(d.status, Status::Patch | Status::Minor | Status::Major)) {
                    outdated += 1;
                }
                let Some(worst) = usages
                    .iter()
                    .flat_map(|d| d.vulns.iter().map(move |id| (*d, id)))
                    .filter_map(|(d, id)| advisories.get(id.as_str()).map(|v| (d, *v)))
                    .max_by_key(|(_, v)| rank(v.severity.as_deref()))
                else {
                    continue;
                };
                match rank(worst.1.severity.as_deref()) {
                    4 => severity.critical += 1,
                    3 => severity.high += 1,
                    2 => severity.moderate += 1,
                    1 => severity.low += 1,
                    _ => severity.unknown += 1,
                }
                let (dep, advisory) = worst;
                let fixed_in = usages.iter().filter(|d| !d.vulns.is_empty()).find_map(|d| d.fix_target.clone());
                problems.push(Problem {
                    name: name.clone(),
                    ecosystem: ecosystem.clone(),
                    version: dep.current.clone().or_else(|| dep.installed.clone()),
                    fixed_in,
                    severity: advisory.severity.as_deref().map(|s| if s == "MEDIUM" { "MODERATE".to_string() } else { s.to_string() }),
                    summary: advisory.summary.clone(),
                    advisory: advisory.id.clone(),
                    url: advisory.url.clone(),
                });
            }
            let vulnerable = problems.len() as u32;
            let fixable = problems.iter().filter(|p| p.fixed_in.is_some()).count() as u32;
            problems.sort_by(|a, b| {
                b.fixed_in.is_some()
                    .cmp(&a.fixed_in.is_some())
                    .then_with(|| rank(b.severity.as_deref()).cmp(&rank(a.severity.as_deref())))
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            problems.truncate(PROBLEMS_KEPT);
            RepoStatus { path, checked_at, checked_commit, vulnerable, fixable, severity, outdated, problems }
        })
        .collect();

    StatusFile { format: FORMAT, written_by: written_by.to_string(), exe: exe.map(str::to_string), written_at: now(), full_check_at, background_check: true, repos }
}

/// The file in `dir`, when there is one Mehen can read.
pub fn read(dir: &Path) -> Option<StatusFile> {
    let file: StatusFile = serde_json::from_str(&fs::read_to_string(dir.join(FILE_NAME)).ok()?).ok()?;
    (file.format == FORMAT).then_some(file)
}

/// Rewrites the file in `dir` from `inventory`. Written beside the real file
/// and renamed over it, so a reader never sees half a file.
pub fn write(dir: &Path, inventory: &Inventory, fresh: Fresh, written_by: &str, exe: Option<&str>) -> anyhow::Result<()> {
    let status = build(inventory, fresh, read(dir).as_ref(), written_by, exe);
    fs::create_dir_all(dir)?;
    let temp = dir.join(format!("{FILE_NAME}.tmp"));
    fs::write(&temp, serde_json::to_vec_pretty(&status)?)?;
    fs::rename(&temp, dir.join(FILE_NAME))?;
    Ok(())
}

fn key(path: &str) -> String {
    path.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DepKind, Dependency, Ecosystem, Project, Vulnerability};

    fn dep(name: &str, status: Status, vulns: &[&str]) -> Dependency {
        let mut d = Dependency::new(name, Ecosystem::Npm, DepKind::Normal, "1.0.0");
        d.current = Some("1.0.0".into());
        d.status = status;
        d.vulns = vulns.iter().map(|v| v.to_string()).collect();
        d
    }

    fn project(dir: &str, repo: Option<&str>, deps: Vec<Dependency>) -> Project {
        Project {
            id: format!("{dir}\\package.json"),
            name: dir.into(),
            ecosystem: Ecosystem::Npm,
            dir: dir.into(),
            manifest: format!("{dir}\\package.json"),
            repo: repo.map(str::to_string),
            frameworks: Vec::new(),
            rust_version: None,
            node_version: None,
            node_engines: None,
            python_version: None,
            php_version: None,
            dependencies: deps,
        }
    }

    fn vuln(id: &str, severity: Option<&str>) -> Vulnerability {
        Vulnerability { id: id.into(), aliases: Vec::new(), summary: format!("{id} summary"), severity: severity.map(str::to_string), url: String::new(), fixed: Vec::new() }
    }

    fn inventory(projects: Vec<Project>, vulns: Vec<Vulnerability>, at: u64) -> Inventory {
        Inventory { roots: vec!["C:\\code".into()], projects, vulnerabilities: vulns, checked_at: Some(at), ..Default::default() }
    }

    #[test]
    fn counts_each_package_once_per_repository_by_its_worst_advisory() {
        let inv = inventory(
            vec![
                project("C:\\code\\app\\web", Some("C:\\code\\app"), vec![dep("lodash", Status::Minor, &["LOW-1", "HIGH-1"]), dep("react", Status::Major, &[]), dep("zod", Status::UpToDate, &[])]),
                // The same package in a second project of the same repo counts once.
                project("C:\\code\\app\\api", Some("C:\\code\\app"), vec![dep("lodash", Status::Minor, &["HIGH-1"]), dep("left-pad", Status::Patch, &["NONE-1"])]),
                // Not in a git repository: left out.
                project("C:\\code\\loose", None, vec![dep("lodash", Status::Minor, &["HIGH-1"])]),
            ],
            vec![vuln("LOW-1", Some("LOW")), vuln("HIGH-1", Some("HIGH")), vuln("NONE-1", None)],
            100,
        );
        let file = build(&inv, Fresh::All, None, "Mehen test", None);
        assert_eq!(file.repos.len(), 1);
        let repo = &file.repos[0];
        assert_eq!(repo.path, "C:\\code\\app");
        assert_eq!(repo.vulnerable, 2);
        assert_eq!(repo.fixable, 0);
        assert_eq!(repo.outdated, 3);
        assert_eq!(repo.severity, SeverityCounts { high: 1, unknown: 1, ..Default::default() });
        let names: Vec<&str> = repo.problems.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["lodash", "left-pad"]);
        assert_eq!(repo.problems[0].advisory, "HIGH-1");
        assert_eq!(repo.checked_at, Some(100));
    }

    #[test]
    fn lists_packages_with_a_fix_first_and_counts_them() {
        let mut fixed = dep("fixed", Status::Minor, &["LOW-1"]);
        fixed.fix_target = Some("1.0.1".into());
        let inv = inventory(
            vec![project("C:\\code\\a", Some("C:\\code\\a"), vec![dep("stuck", Status::UpToDate, &["HIGH-1"]), fixed])],
            vec![vuln("LOW-1", Some("LOW")), vuln("HIGH-1", Some("HIGH"))],
            1,
        );
        let repo = &build(&inv, Fresh::All, None, "Mehen test", None).repos[0];
        assert_eq!((repo.vulnerable, repo.fixable), (2, 1));
        assert_eq!(repo.problems[0].name, "fixed");
        assert_eq!(repo.problems[0].fixed_in.as_deref(), Some("1.0.1"));
        assert_eq!(repo.problems[1].fixed_in, None);
    }

    #[test]
    fn a_partial_check_keeps_the_time_and_commit_of_repositories_it_did_not_check() {
        let previous = StatusFile {
            format: FORMAT,
            written_by: String::new(),
            exe: None,
            written_at: 0,
            full_check_at: Some(40),
            background_check: true,
            repos: vec![RepoStatus {
                path: "c:/code/b".into(),
                checked_at: Some(50),
                checked_commit: Some("abc".into()),
                vulnerable: 0,
                fixable: 0,
                severity: SeverityCounts::default(),
                outdated: 0,
                problems: Vec::new(),
            }],
        };
        let inv = inventory(
            vec![project("C:\\code\\a", Some("C:\\code\\a"), vec![dep("x", Status::UpToDate, &[])]), project("C:\\code\\b", Some("C:\\code\\b"), vec![dep("y", Status::UpToDate, &[])])],
            Vec::new(),
            100,
        );
        let file = build(&inv, Fresh::Only(&["C:\\code\\a".into()]), Some(&previous), "Mehen test", None);
        let b = file.repos.iter().find(|r| r.path == "C:\\code\\b").unwrap();
        assert_eq!((b.checked_at, b.checked_commit.as_deref()), (Some(50), Some("abc")));
        let a = file.repos.iter().find(|r| r.path == "C:\\code\\a").unwrap();
        assert_eq!(a.checked_at, Some(100));

        assert_eq!(file.full_check_at, Some(40), "a partial check keeps the last full check's time");
        let unchanged = build(&inv, Fresh::None, Some(&previous), "Mehen test", None);
        let a = unchanged.repos.iter().find(|r| r.path == "C:\\code\\a").unwrap();
        assert_eq!((a.checked_at, a.checked_commit.as_deref()), (Some(100), None));
    }

    #[test]
    fn reads_the_checked_out_commit_from_loose_and_packed_refs() {
        let root = std::env::temp_dir().join("mehen-status-head");
        let _ = fs::remove_dir_all(&root);
        let git = root.join(".git");
        fs::create_dir_all(git.join("refs/heads")).unwrap();
        let sha = "0123456789abcdef0123456789abcdef01234567";
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(git.join("refs/heads/main"), format!("{sha}\n")).unwrap();
        assert_eq!(head_commit(&root).as_deref(), Some(sha));

        fs::remove_file(git.join("refs/heads/main")).unwrap();
        let packed = "fedcba9876543210fedcba9876543210fedcba98";
        fs::write(git.join("packed-refs"), format!("# pack-refs with: peeled\n{packed} refs/heads/main\n")).unwrap();
        assert_eq!(head_commit(&root).as_deref(), Some(packed));

        fs::write(git.join("HEAD"), format!("{sha}\n")).unwrap();
        assert_eq!(head_commit(&root).as_deref(), Some(sha));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn writes_a_file_it_can_read_back() {
        let dir = std::env::temp_dir().join("mehen-status-write");
        let _ = fs::remove_dir_all(&dir);
        let inv = inventory(vec![project("C:\\code\\a", Some("C:\\code\\a"), vec![dep("x", Status::Major, &[])])], Vec::new(), 7);
        write(&dir, &inv, Fresh::All, "Mehen test", Some("C:\\mehen.exe")).unwrap();
        let back = read(&dir).unwrap();
        assert_eq!(back.repos[0].outdated, 1);
        assert_eq!(back.exe.as_deref(), Some("C:\\mehen.exe"));
        assert!(!dir.join(format!("{FILE_NAME}.tmp")).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
