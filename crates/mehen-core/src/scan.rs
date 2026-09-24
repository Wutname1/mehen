//! Walks a folder tree and reads every manifest it understands.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use ignore::WalkBuilder;
use regex::Regex;

use crate::ignore::IgnoreSet;
use crate::lockfiles::LockfileCache;
use crate::model::{DepKind, Dependency, Ecosystem, Inventory, Project, Status};
use crate::version::{Version, from_spec};

const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", "bin", "obj", ".git", "dist", "build", "out", ".next", ".nuxt", ".turbo", ".svelte-kit", ".venv", "venv",
    ".vs", ".idea", "coverage", ".dart_tool", ".gradle", "Pods", ".pnpm-store", ".yarn",
];

static USES_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^[\s-]*uses:\s*["']?([^"'\s#]+)["']?[ \t]*(?:#[ \t]*(\S+))?"#).unwrap());

/// Folders nested inside another watched folder would be walked twice.
fn distinct_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let existing: Vec<&PathBuf> = roots.iter().filter(|r| r.is_dir()).collect();
    existing
        .iter()
        .filter(|r| !existing.iter().any(|other| other != *r && r.starts_with(other)))
        .map(|r| (*r).clone())
        .collect()
}

pub fn scan(roots: &[PathBuf], ignore: &IgnoreSet) -> Inventory {
    let start = Instant::now();
    let roots = distinct_roots(roots);
    let skipped = Arc::new(Mutex::new(Vec::new()));
    let ignored = Arc::new(Mutex::new(Vec::new()));

    let mut scanner = Scanner { roots: roots.clone(), ..Default::default() };
    let mut workflows: BTreeMap<PathBuf, Vec<(Dependency, PathBuf)>> = BTreeMap::new();
    let Some((first, rest)) = roots.split_first() else {
        return scanner.finish(roots, Vec::new(), Vec::new(), start);
    };

    let mut builder = WalkBuilder::new(first);
    for root in rest {
        builder.add(root);
    }
    let (skipped_in_filter, ignored_in_filter, rules) = (Arc::clone(&skipped), Arc::clone(&ignored), ignore.clone());
    let walker = builder
        .hidden(false)
        .follow_links(false)
        .filter_entry(move |entry| {
            if !entry.file_type().is_some_and(|t| t.is_dir()) {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            if SKIP_DIRS.iter().any(|d| d.eq_ignore_ascii_case(&name)) {
                return false;
            }
            if entry.depth() > 0 && rules.matching_rule(entry.path()).is_some() {
                ignored_in_filter.lock().unwrap().push(entry.path().display().to_string());
                return false;
            }
            if entry.depth() > 0 && is_linked_worktree(entry.path()) {
                skipped_in_filter.lock().unwrap().push(entry.path().display().to_string());
                return false;
            }
            true
        })
        .build_parallel();

    // Walking is most of a scan and waits on the disk, so it runs on several
    // threads; the few manifests it finds are read in order afterwards.
    let found: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    walker.run(|| {
        let found = &found;
        Box::new(move |entry| {
            if let Ok(entry) = entry {
                if entry.file_type().is_some_and(|t| t.is_file()) {
                    let file_name = entry.file_name().to_string_lossy();
                    let ext = entry.path().extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                    if is_manifest(&file_name, &ext) {
                        found.lock().unwrap().push(entry.into_path());
                    }
                }
            }
            ignore::WalkState::Continue
        })
    });
    let mut found = found.into_inner().unwrap();
    found.sort();

    for path in &found {
        let path = path.as_path();
        let file_name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ignore.matching_rule(path).is_some() {
            ignored.lock().unwrap().push(path.display().to_string());
            continue;
        }

        let result = match file_name.as_ref() {
            "package.json" => scanner.package_json(path),
            "Cargo.toml" => scanner.cargo_toml(path),
            "go.mod" => scanner.go_mod(path),
            "pubspec.yaml" => scanner.pubspec(path),
            "pyproject.toml" => scanner.pyproject(path),
            "Pipfile" => scanner.pipfile(path),
            _ if file_name.starts_with("requirements") && ext == "txt" => scanner.requirements_txt(path),
            "packages.config" => scanner.packages_config(path),
            "Directory.Packages.props" => scanner.central_packages(path),
            _ if matches!(ext.as_str(), "csproj" | "fsproj" | "vbproj") => scanner.msbuild_project(path),
            _ if is_workflow_file(path, &file_name, &ext) => {
                let owner = workflow_owner(path);
                for dep in read_workflow(path) {
                    workflows.entry(owner.clone()).or_default().push((dep, path.to_path_buf()));
                }
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            scanner.warnings.push(format!("{}: {e}", path.display()));
        }
    }

    for (dir, deps) in workflows {
        if ignore.matching_rule(&dir.join(".github")).is_none() {
            scanner.push_workflow_project(&dir, deps);
        }
    }

    let skipped_worktrees = std::mem::take(&mut *skipped.lock().unwrap());
    let ignored = std::mem::take(&mut *ignored.lock().unwrap());
    scanner.finish(roots, skipped_worktrees, ignored, start)
}

/// Every project under the roots, ignored or not, each marked with the rule
/// that hides it. Makes no network calls, so it is cheap to run before a check.
pub fn discover(roots: &[PathBuf], rules: &[crate::ignore::IgnoreRule]) -> Vec<crate::model::DiscoveredProject> {
    let everything = scan(roots, &IgnoreSet::default());
    let set = IgnoreSet::new(rules, roots);
    everything
        .projects
        .into_iter()
        .map(|p| crate::model::DiscoveredProject {
            ignored_by: set.matching_rule(Path::new(&p.manifest)).or_else(|| set.matching_rule(Path::new(&p.dir))),
            dependency_count: p.dependencies.len(),
            id: p.id,
            name: p.name,
            ecosystem: p.ecosystem,
            dir: p.dir,
            manifest: p.manifest,
            repo: p.repo,
        })
        .collect()
}

/// A package listed twice (say, in two dependency groups with the same
/// spec) shows once.
fn dedupe_python(deps: &mut Vec<Dependency>) {
    let mut seen = std::collections::HashSet::new();
    deps.retain(|d| seen.insert((crate::python::normalize(&d.name), d.requested.clone())));
}

fn is_manifest(file_name: &str, ext: &str) -> bool {
    matches!(file_name, "package.json" | "Cargo.toml" | "packages.config" | "Directory.Packages.props" | "go.mod" | "pyproject.toml" | "Pipfile" | "pubspec.yaml")
        || file_name.starts_with("requirements") && ext == "txt"
        || matches!(ext, "csproj" | "fsproj" | "vbproj" | "yml" | "yaml")
}

/// A linked worktree has a `.git` file pointing into another repo's `worktrees/`
/// folder. Scanning it would count every dependency in that repo twice.
fn is_linked_worktree(dir: &Path) -> bool {
    let git = dir.join(".git");
    git.is_file() && fs::read_to_string(&git).is_ok_and(|s| s.contains("worktrees"))
}

fn is_workflow_file(path: &Path, file_name: &str, ext: &str) -> bool {
    if !matches!(ext, "yml" | "yaml") {
        return false;
    }
    if file_name == "action.yml" || file_name == "action.yaml" {
        return true;
    }
    let parent = path.parent();
    parent.and_then(|p| p.file_name()).is_some_and(|n| n == "workflows")
        && parent.and_then(|p| p.parent()).and_then(|p| p.file_name()).is_some_and(|n| n == ".github")
}

/// Workflows group under the folder that owns `.github`; composite actions
/// group under their repo.
fn workflow_owner(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(path);
    if dir.file_name().is_some_and(|n| n == "workflows") {
        if let Some(owner) = dir.parent().and_then(|p| p.parent()) {
            return owner.to_path_buf();
        }
    }
    find_repo(dir).unwrap_or_else(|| dir.to_path_buf())
}

fn read_workflow(path: &Path) -> Vec<Dependency> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    USES_RE
        .captures_iter(&text)
        .filter_map(|cap| {
            let full = cap.get(1)?.as_str();
            if full.starts_with("./") || full.starts_with("docker://") {
                return None;
            }
            let (target, reference) = full.split_once('@')?;
            let mut segments = target.splitn(3, '/');
            let owner = segments.next()?;
            let repo = segments.next()?;
            let mut dep = Dependency::new(format!("{owner}/{repo}"), Ecosystem::GithubActions, DepKind::Action, reference);
            dep.pinned_comment = cap.get(2).map(|m| m.as_str().to_string()).filter(|c| Version::parse(c).is_some());
            if let Some(sub) = segments.next() {
                dep.note = Some(format!("uses {sub}"));
            }
            Some(dep)
        })
        .collect()
}

fn find_repo(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().find(|d| d.join(".git").exists()).map(Path::to_path_buf)
}

fn dir_name(dir: &Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string())
}

fn read_text(path: &Path) -> anyhow::Result<String> {
    let text = fs::read_to_string(path)?;
    Ok(text.trim_start_matches('\u{feff}').to_string())
}

#[derive(Default)]
struct Scanner {
    roots: Vec<PathBuf>,
    projects: Vec<Project>,
    warnings: Vec<String>,
    cargo_locks: HashMap<PathBuf, HashMap<String, Vec<String>>>,
    npm_locks: LockfileCache,
}

impl Scanner {
    fn finish(mut self, roots: Vec<PathBuf>, skipped_worktrees: Vec<String>, ignored: Vec<String>, start: Instant) -> Inventory {
        self.projects.sort_by(|a, b| a.dir.to_lowercase().cmp(&b.dir.to_lowercase()).then(a.name.cmp(&b.name)));
        Inventory {
            roots: roots.iter().map(|r| r.display().to_string()).collect(),
            projects: self.projects,
            vulnerabilities: Vec::new(),
            skipped_worktrees,
            ignored,
            warnings: self.warnings,
            scan_ms: start.elapsed().as_millis() as u64,
            check_ms: None,
            checked_at: None,
            check_stats: None,
        }
    }

    /// The watched folder containing `dir`; lockfile searches stop there.
    fn root_of(&self, dir: &Path) -> PathBuf {
        self.roots.iter().find(|r| dir.starts_with(r)).cloned().unwrap_or_else(|| dir.to_path_buf())
    }

    fn push(&mut self, manifest: &Path, name: String, ecosystem: Ecosystem, frameworks: Vec<String>, dependencies: Vec<Dependency>) -> Option<&mut Project> {
        if dependencies.is_empty() {
            return None;
        }
        let dir = manifest.parent().unwrap_or(manifest);
        self.projects.push(Project {
            id: manifest.display().to_string(),
            name,
            ecosystem,
            dir: dir.display().to_string(),
            manifest: manifest.display().to_string(),
            repo: find_repo(dir).map(|r| r.display().to_string()),
            frameworks,
            rust_version: None,
            node_version: None,
            node_engines: None,
            python_version: None,
            dependencies,
        });
        self.projects.last_mut()
    }

    fn push_workflow_project(&mut self, dir: &Path, found: Vec<(Dependency, PathBuf)>) {
        let mut merged: Vec<Dependency> = Vec::new();
        let mut files: HashMap<(String, String), Vec<String>> = HashMap::new();
        for (dep, file) in found {
            let key = (dep.name.clone(), dep.requested.clone());
            let file_name = dir_name(&file);
            let seen = files.entry(key).or_default();
            if seen.is_empty() {
                merged.push(dep);
            }
            if !seen.contains(&file_name) {
                seen.push(file_name);
            }
        }
        for dep in &mut merged {
            let used_in = &files[&(dep.name.clone(), dep.requested.clone())];
            let where_used = format!("in {}", used_in.join(", "));
            dep.note = Some(match dep.note.take() {
                Some(n) => format!("{n}; {where_used}"),
                None => where_used,
            });
        }
        let manifest = dir.join(".github");
        let name = format!("{} (Actions)", dir_name(dir));
        if merged.is_empty() {
            return;
        }
        self.projects.push(Project {
            id: manifest.display().to_string(),
            name,
            ecosystem: Ecosystem::GithubActions,
            dir: dir.display().to_string(),
            manifest: manifest.display().to_string(),
            repo: find_repo(dir).map(|r| r.display().to_string()),
            frameworks: Vec::new(),
            rust_version: None,
            node_version: None,
            node_engines: None,
            python_version: None,
            dependencies: merged,
        });
    }

    fn package_json(&mut self, path: &Path) -> anyhow::Result<()> {
        let json: serde_json::Value = serde_json::from_str(&read_text(path)?)?;
        let dir = path.parent().unwrap_or(path);
        let name = json["name"].as_str().map(str::to_string).unwrap_or_else(|| dir_name(dir));
        let mut deps = Vec::new();
        for (field, kind) in [
            ("dependencies", DepKind::Normal),
            ("devDependencies", DepKind::Dev),
            ("peerDependencies", DepKind::Peer),
            ("optionalDependencies", DepKind::Normal),
        ] {
            let Some(map) = json[field].as_object() else { continue };
            for (dep_name, spec) in map {
                let spec = spec.as_str().unwrap_or("").to_string();
                let mut dep = Dependency::new(dep_name, Ecosystem::Npm, kind, &spec);
                if let Some(reason) = npm_local_reason(&spec) {
                    dep.status = Status::Local;
                    dep.note = Some(reason.into());
                } else {
                    if let Some((version, source)) = self.npm_installed(dir, dep_name) {
                        dep.installed = Some(version);
                        dep.installed_from = Some(source.into());
                    }
                }
                deps.push(dep);
            }
        }
        let engines = json["engines"]["node"].as_str().map(str::to_string);
        let pinned = self.node_version_file(dir);
        if let Some(project) = self.push(path, name, Ecosystem::Npm, Vec::new(), deps) {
            project.node_version = pinned;
            project.node_engines = engines;
        }
        Ok(())
    }

    /// `.nvmrc` or `.node-version` in the project or a parent, when it names a
    /// version rather than an alias like `lts/*`.
    fn node_version_file(&self, dir: &Path) -> Option<String> {
        let root = self.root_of(dir);
        dir.ancestors().take_while(|d| d.starts_with(&root)).find_map(|d| {
            [".nvmrc", ".node-version"].iter().find_map(|f| {
                let text = fs::read_to_string(d.join(f)).ok()?;
                let v = text.trim().trim_start_matches(['v', 'V']);
                v.starts_with(|c: char| c.is_ascii_digit()).then(|| v.to_string())
            })
        })
    }

    fn npm_installed(&mut self, dir: &Path, name: &str) -> Option<(String, &'static str)> {
        let root = self.root_of(dir);
        let from_node_modules = dir.ancestors().take_while(|d| d.starts_with(&root)).find_map(|d| {
            let manifest = d.join("node_modules").join(name).join("package.json");
            let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(manifest).ok()?).ok()?;
            json["version"].as_str().map(str::to_string)
        });
        match from_node_modules {
            Some(v) => Some((v, "node_modules")),
            None => self.npm_locks.npm_version(&root, dir, name),
        }
    }

    /// Direct requirements only: `// indirect` ones come along with them, as
    /// npm's nested packages do. A replaced module is local, since updating
    /// the original would change nothing.
    /// Versions come from `pubspec.lock`; the Flutter SDK, folders, git and
    /// other package servers are local.
    fn pubspec(&mut self, path: &Path) -> anyhow::Result<()> {
        let spec = crate::dart::parse(&read_text(path)?)?;
        let dir = path.parent().unwrap_or(path);
        let lock = dir.join("pubspec.lock");
        let locked = crate::dart::read_lock(&lock);
        let mut deps = Vec::new();
        for entry in &spec.entries {
            let kind = if entry.dev { DepKind::Dev } else { DepKind::Normal };
            let mut dep = Dependency::new(&entry.name, Ecosystem::Pub, kind, &entry.constraint);
            match (entry.local, locked.get(&entry.name)) {
                (Some(why), _) => {
                    dep.status = Status::Local;
                    dep.note = Some(why.into());
                }
                (None, Some(version)) => {
                    dep.installed = Some(version.clone());
                    dep.installed_from = Some("pubspec.lock".into());
                }
                (None, None) if entry.constraint.is_empty() || entry.constraint == "any" => dep.note = Some("no version given".into()),
                (None, None) => {}
            }
            deps.push(dep);
        }
        let name = spec.name.clone().unwrap_or_else(|| dir_name(dir));
        self.push(path, name, Ecosystem::Pub, Vec::new(), deps);
        Ok(())
    }

    fn go_mod(&mut self, path: &Path) -> anyhow::Result<()> {
        let parsed = crate::golang::parse(&read_text(path)?);
        let dir = path.parent().unwrap_or(path);
        let mut deps = Vec::new();
        for require in parsed.requires.iter().filter(|r| !r.indirect) {
            let mut dep = Dependency::new(&require.path, Ecosystem::Go, DepKind::Normal, &require.version);
            match parsed.replaced.iter().find(|(from, _)| *from == require.path) {
                Some((_, to)) => {
                    dep.status = Status::Local;
                    dep.note = Some(format!("replaced by {to}"));
                }
                None => {
                    dep.installed = Some(require.version.clone());
                    dep.installed_from = Some("go.mod".into());
                }
            }
            deps.push(dep);
        }
        let name = parsed.module.as_deref().map(crate::golang::module_name).unwrap_or_else(|| dir_name(dir));
        self.push(path, name, Ecosystem::Go, Vec::new(), deps);
        Ok(())
    }

    /// A Python dependency, versioned by the lockfile when there is one, else
    /// by an exact `==` pin (or Poetry's bare version).
    fn python_dep(name: &str, spec: &str, kind: DepKind, locked: &HashMap<String, String>, lock_name: Option<&str>, source: &str) -> Dependency {
        let mut dep = Dependency::new(name, Ecosystem::Pypi, kind, spec);
        let s = spec.trim();
        let exact = s.strip_prefix("==").unwrap_or(if s.starts_with(|c: char| c.is_ascii_digit()) { s } else { "" }).trim();
        let pinned = (!exact.is_empty() && !exact.contains(['*', ',']) && crate::python::release(exact).is_some()).then_some(exact);
        match (locked.get(&crate::python::normalize(name)), pinned) {
            (Some(v), _) => {
                dep.installed = Some(v.clone());
                dep.installed_from = lock_name.map(Into::into);
            }
            (None, Some(v)) => {
                dep.installed = Some(v.to_string());
                dep.installed_from = Some(source.into());
            }
            (None, None) if s.is_empty() || s == "*" => dep.note = Some("no version given".into()),
            (None, None) => {}
        }
        dep
    }

    fn python_local(name: &str, kind: DepKind, why: &str) -> Dependency {
        let mut dep = Dependency::new(name, Ecosystem::Pypi, kind, "");
        dep.status = Status::Local;
        dep.note = Some(why.into());
        dep
    }

    /// The lowest Python in `.python-version`, next to the manifest.
    fn python_version_file(dir: &Path) -> Option<String> {
        let text = fs::read_to_string(dir.join(".python-version")).ok()?;
        crate::python::lowest_python(text.lines().next()?.trim())
    }

    fn requirements_txt(&mut self, path: &Path) -> anyhow::Result<()> {
        let dir = path.parent().unwrap_or(path);
        let file = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let kind = if file.contains("dev") || file.contains("test") { DepKind::Dev } else { DepKind::Normal };
        let mut deps = Vec::new();
        for req in read_text(path)?.lines().filter_map(crate::python::parse_requirement) {
            deps.push(if req.direct { Self::python_local(&req.name, kind, "installed from a URL or folder") } else { Self::python_dep(&req.name, &req.spec, kind, &HashMap::new(), None, &file) });
        }
        let name = if file == "requirements.txt" { dir_name(dir) } else { format!("{} ({file})", dir_name(dir)) };
        if let Some(project) = self.push(path, name, Ecosystem::Pypi, Vec::new(), deps) {
            project.python_version = Self::python_version_file(dir);
        }
        Ok(())
    }

    fn pyproject(&mut self, path: &Path) -> anyhow::Result<()> {
        let table: toml::Table = toml::from_str(&read_text(path)?)?;
        let dir = path.parent().unwrap_or(path);
        let (locked, lock_name) = match crate::python::Manager::for_manifest(path) {
            Some((_, lock)) => (crate::python::read_lock(&lock), lock.file_name().map(|f| f.to_string_lossy().to_string())),
            None => (HashMap::new(), None),
        };
        let project = table.get("project");
        let tool = table.get("tool");
        let poetry = tool.and_then(|t| t.get("poetry"));
        let mut deps: Vec<Dependency> = Vec::new();
        let add_strings = |value: Option<&toml::Value>, kind: DepKind, deps: &mut Vec<Dependency>| {
            for req in value.and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str()).filter_map(crate::python::parse_requirement) {
                deps.push(if req.direct { Self::python_local(&req.name, kind, "installed from a URL or folder") } else { Self::python_dep(&req.name, &req.spec, kind, &locked, lock_name.as_deref(), "pyproject.toml") });
            }
        };
        add_strings(project.and_then(|p| p.get("dependencies")), DepKind::Normal, &mut deps);
        for groups in [project.and_then(|p| p.get("optional-dependencies")), table.get("dependency-groups"), tool.and_then(|t| t.get("pdm")).and_then(|p| p.get("dev-dependencies"))] {
            for group in groups.and_then(|g| g.as_table()).into_iter().flat_map(|t| t.values()) {
                add_strings(Some(group), DepKind::Dev, &mut deps);
            }
        }
        let mut tables: Vec<(&toml::Table, DepKind)> = Vec::new();
        if let Some(p) = poetry {
            tables.extend(p.get("dependencies").and_then(|d| d.as_table()).map(|t| (t, DepKind::Normal)));
            tables.extend(p.get("dev-dependencies").and_then(|d| d.as_table()).map(|t| (t, DepKind::Dev)));
            for group in p.get("group").and_then(|g| g.as_table()).into_iter().flat_map(|t| t.values()) {
                tables.extend(group.get("dependencies").and_then(|d| d.as_table()).map(|t| (t, DepKind::Dev)));
            }
        }
        for (entries, kind) in tables {
            for (name, value) in entries.iter().filter(|(k, _)| k.as_str() != "python") {
                deps.push(Self::python_table_dep(name, value, kind, &locked, lock_name.as_deref(), "pyproject.toml"));
            }
        }
        dedupe_python(&mut deps);

        let name = project.and_then(|p| p.get("name")).or_else(|| poetry.and_then(|p| p.get("name"))).and_then(|n| n.as_str()).map(str::to_string).unwrap_or_else(|| dir_name(dir));
        let python = project
            .and_then(|p| p.get("requires-python"))
            .or_else(|| poetry.and_then(|p| p.get("dependencies")).and_then(|d| d.get("python")))
            .and_then(|v| v.as_str())
            .and_then(crate::python::lowest_python)
            .or_else(|| Self::python_version_file(dir));
        if let Some(project) = self.push(path, name, Ecosystem::Pypi, Vec::new(), deps) {
            project.python_version = python;
        }
        Ok(())
    }

    /// A `name = spec` entry, as Poetry and Pipenv write them: a string, or a
    /// table with `version` (or `path`, `git`, `url` for local and direct ones).
    fn python_table_dep(name: &str, value: &toml::Value, kind: DepKind, locked: &HashMap<String, String>, lock_name: Option<&str>, source: &str) -> Dependency {
        let table = value.as_table().or_else(|| value.as_array().and_then(|a| a.first()).and_then(|v| v.as_table()));
        match (value.as_str(), table) {
            (Some(spec), _) => Self::python_dep(name, spec, kind, locked, lock_name, source),
            (None, Some(t)) if ["path", "git", "url", "file", "editable"].iter().any(|k| t.contains_key(*k)) && !t.contains_key("version") => {
                Self::python_local(name, kind, "installed from a folder, URL or git")
            }
            (None, Some(t)) => Self::python_dep(name, t.get("version").and_then(|v| v.as_str()).unwrap_or(""), kind, locked, lock_name, source),
            (None, None) => Self::python_local(name, kind, "unrecognised entry"),
        }
    }

    fn pipfile(&mut self, path: &Path) -> anyhow::Result<()> {
        let table: toml::Table = toml::from_str(&read_text(path)?)?;
        let dir = path.parent().unwrap_or(path);
        let lock = dir.join("Pipfile.lock");
        let locked = crate::python::read_lock(&lock);
        let lock_name = lock.is_file().then_some("Pipfile.lock");
        let mut deps = Vec::new();
        for (section, kind) in [("packages", DepKind::Normal), ("dev-packages", DepKind::Dev)] {
            for (name, value) in table.get(section).and_then(|s| s.as_table()).into_iter().flatten() {
                deps.push(Self::python_table_dep(name, value, kind, &locked, lock_name, "Pipfile"));
            }
        }
        dedupe_python(&mut deps);
        let python = table.get("requires").and_then(|r| r.get("python_version").or_else(|| r.get("python_full_version"))).and_then(|v| v.as_str()).map(str::to_string);
        if let Some(project) = self.push(path, format!("{} (Pipfile)", dir_name(dir)), Ecosystem::Pypi, Vec::new(), deps) {
            project.python_version = python.or_else(|| Self::python_version_file(dir));
        }
        Ok(())
    }

    fn cargo_toml(&mut self, path: &Path) -> anyhow::Result<()> {
        let table: toml::Table = toml::from_str(&read_text(path)?)?;
        let dir = path.parent().unwrap_or(path);
        let package_name = table.get("package").and_then(|p| p.get("name")).and_then(|n| n.as_str());

        let mut deps = Vec::new();
        let mut sections: Vec<(&toml::Table, DepKind)> = Vec::new();
        for (key, kind) in [("dependencies", DepKind::Normal), ("dev-dependencies", DepKind::Dev), ("build-dependencies", DepKind::Build)] {
            if let Some(t) = table.get(key).and_then(|v| v.as_table()) {
                sections.push((t, kind));
            }
            if let Some(targets) = table.get("target").and_then(|v| v.as_table()) {
                for target in targets.values().filter_map(|v| v.as_table()) {
                    if let Some(t) = target.get(key).and_then(|v| v.as_table()) {
                        sections.push((t, kind));
                    }
                }
            }
        }
        if let Some(t) = table.get("workspace").and_then(|w| w.get("dependencies")).and_then(|v| v.as_table()) {
            sections.push((t, DepKind::Normal));
        }

        for (section, kind) in sections {
            for (key, value) in section {
                deps.push(self.cargo_dep(dir, key, value, kind));
            }
        }
        let name = match package_name {
            Some(n) => n.to_string(),
            None => format!("{} (workspace)", dir_name(dir)),
        };
        let rust = rust_version(&table).or_else(|| {
            // `rust-version.workspace = true` (or a workspace-only manifest):
            // the value lives in [workspace.package] further up.
            let root = self.root_of(dir);
            dir.ancestors().skip(1).take_while(|d| d.starts_with(&root)).find_map(|d| {
                let t: toml::Table = toml::from_str(&fs::read_to_string(d.join("Cargo.toml")).ok()?).ok()?;
                t.get("workspace")?.get("package")?.get("rust-version")?.as_str().map(str::to_string)
            })
        });
        if let Some(project) = self.push(path, name, Ecosystem::Cargo, Vec::new(), deps) {
            project.rust_version = rust;
        }
        Ok(())
    }

    fn cargo_dep(&mut self, dir: &Path, key: &str, value: &toml::Value, kind: DepKind) -> Dependency {
        let (name, spec, local) = match value {
            toml::Value::String(s) => (key.to_string(), s.clone(), None),
            toml::Value::Table(t) => {
                let name = t.get("package").and_then(|p| p.as_str()).unwrap_or(key).to_string();
                let version = t.get("version").and_then(|v| v.as_str()).map(str::to_string);
                let local = if t.get("workspace").and_then(|w| w.as_bool()) == Some(true) {
                    Some("inherits the workspace version")
                } else if t.contains_key("git") {
                    Some("git dependency")
                } else if t.contains_key("path") && version.is_none() {
                    Some("local path")
                } else {
                    None
                };
                (name, version.unwrap_or_default(), local)
            }
            _ => (key.to_string(), String::new(), Some("unrecognised entry")),
        };
        let mut dep = Dependency::new(name, Ecosystem::Cargo, kind, &spec);
        match local {
            Some(reason) => {
                dep.status = Status::Local;
                dep.note = Some(reason.into());
            }
            None => {
                dep.installed = self.cargo_locked(dir, &dep.name, &spec);
                dep.installed_from = dep.installed.as_ref().map(|_| "Cargo.lock".into());
            }
        }
        dep
    }

    fn cargo_locked(&mut self, dir: &Path, name: &str, spec: &str) -> Option<String> {
        let root = self.root_of(dir);
        let lock_path = dir.ancestors().take_while(|d| d.starts_with(&root)).map(|d| d.join("Cargo.lock")).find(|p| p.is_file())?;
        let lock = self.cargo_locks.entry(lock_path.clone()).or_insert_with(|| read_cargo_lock(&lock_path));
        let versions = lock.get(name)?;
        let wanted = from_spec(spec).and_then(|s| Version::parse(&s));
        let compatible = |v: &Version| match &wanted {
            Some(w) if w.parts.first() == Some(&0) => v.parts.first() == Some(&0) && v.parts.get(1) == w.parts.get(1),
            Some(w) => v.parts.first() == w.parts.first(),
            None => true,
        };
        let parsed: Vec<(Version, &String)> = versions.iter().filter_map(|v| Version::parse(v).map(|p| (p, v))).collect();
        parsed
            .iter()
            .filter(|(v, _)| compatible(v))
            .max_by(|a, b| a.0.cmp(&b.0))
            .or_else(|| parsed.iter().max_by(|a, b| a.0.cmp(&b.0)))
            .map(|(_, raw)| raw.to_string())
    }

    fn msbuild_project(&mut self, path: &Path) -> anyhow::Result<()> {
        let text = read_text(path)?;
        let doc = roxmltree::Document::parse(&text)?;
        let mut frameworks = Vec::new();
        let mut deps = Vec::new();
        for node in doc.descendants().filter(|n| n.is_element()) {
            match node.tag_name().name() {
                "TargetFramework" | "TargetFrameworks" | "TargetFrameworkVersion" => {
                    if let Some(t) = node.text() {
                        frameworks.extend(t.split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));
                    }
                }
                "PackageReference" => {
                    let Some(name) = node.attribute("Include").or(node.attribute("Update")) else { continue };
                    let version = node
                        .attribute("Version")
                        .or(node.attribute("VersionOverride"))
                        .map(str::to_string)
                        .or_else(|| node.children().find(|c| c.tag_name().name() == "Version").and_then(|c| c.text()).map(str::to_string));
                    let mut dep = Dependency::new(name, Ecosystem::Nuget, DepKind::Normal, version.clone().unwrap_or_default());
                    if version.is_none() {
                        dep.status = Status::Local;
                        dep.note = Some("version set in Directory.Packages.props".into());
                    }
                    deps.push(dep);
                }
                _ => {}
            }
        }
        frameworks.dedup();
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.push(path, name, Ecosystem::Nuget, frameworks, deps);
        Ok(())
    }

    fn packages_config(&mut self, path: &Path) -> anyhow::Result<()> {
        let text = read_text(path)?;
        let doc = roxmltree::Document::parse(&text)?;
        let mut frameworks = Vec::new();
        let deps = doc
            .descendants()
            .filter(|n| n.tag_name().name() == "package")
            .filter_map(|n| {
                let id = n.attribute("id")?;
                if let Some(tfm) = n.attribute("targetFramework") {
                    if !frameworks.iter().any(|f| f == tfm) {
                        frameworks.push(tfm.to_string());
                    }
                }
                let kind = if n.attribute("developmentDependency") == Some("true") { DepKind::Dev } else { DepKind::Normal };
                Some(Dependency::new(id, Ecosystem::Nuget, kind, n.attribute("version").unwrap_or_default()))
            })
            .collect();
        let dir = path.parent().unwrap_or(path);
        let project_file = fs::read_dir(dir).ok().and_then(|rd| {
            rd.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("csproj") || e.eq_ignore_ascii_case("vbproj")))
        });
        let base = project_file.and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_else(|| dir_name(dir));
        self.push(path, format!("{base} (packages.config)"), Ecosystem::Nuget, frameworks, deps);
        Ok(())
    }

    fn central_packages(&mut self, path: &Path) -> anyhow::Result<()> {
        let text = read_text(path)?;
        let doc = roxmltree::Document::parse(&text)?;
        let deps = doc
            .descendants()
            .filter(|n| n.tag_name().name() == "PackageVersion")
            .filter_map(|n| Some(Dependency::new(n.attribute("Include")?, Ecosystem::Nuget, DepKind::Normal, n.attribute("Version").unwrap_or_default())))
            .collect();
        let dir = path.parent().unwrap_or(path);
        self.push(path, format!("{} (central packages)", dir_name(dir)), Ecosystem::Nuget, Vec::new(), deps);
        Ok(())
    }
}

fn rust_version(table: &toml::Table) -> Option<String> {
    table
        .get("package")
        .and_then(|p| p.get("rust-version"))
        .and_then(|v| v.as_str())
        .or_else(|| table.get("workspace")?.get("package")?.get("rust-version")?.as_str())
        .map(str::to_string)
}

fn npm_local_reason(spec: &str) -> Option<&'static str> {
    let s = spec.trim();
    if s.starts_with("workspace:") {
        Some("workspace package")
    } else if s.starts_with("file:") || s.starts_with("link:") || s.starts_with("portal:") {
        Some("local path")
    } else if s.starts_with("catalog:") {
        Some("pnpm catalog version")
    } else if s.starts_with("npm:") {
        Some("aliased package")
    } else if s.starts_with("git") || s.starts_with("http") || s.contains("github:") || (s.contains('/') && !s.starts_with('@')) {
        Some("git or URL dependency")
    } else {
        None
    }
}

fn read_cargo_lock(path: &Path) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let Some(table) = fs::read_to_string(path).ok().and_then(|t| toml::from_str::<toml::Table>(&t).ok()) else {
        return out;
    };
    for pkg in table.get("package").and_then(|p| p.as_array()).into_iter().flatten() {
        if let (Some(name), Some(version)) = (pkg.get("name").and_then(|v| v.as_str()), pkg.get("version").and_then(|v| v.as_str())) {
            out.entry(name.to_string()).or_default().push(version.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ignore::{IgnoreKind, IgnoreRule};

    #[test]
    fn python_projects_read_locks_pins_and_python_version() {
        let root = std::env::temp_dir().join("mehen-test-python");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("pyproject.toml"),
            "[project]\nname = \"api\"\nrequires-python = \">=3.11\"\ndependencies = [\"httpx>=0.27\", \"Typing_Extensions\", \"shared @ file:///libs/shared\"]\n\n[project.optional-dependencies]\ndocs = [\"mkdocs>=1.5\"]\n",
        )
        .unwrap();
        fs::write(root.join("uv.lock"), "version = 1\n\n[[package]]\nname = \"httpx\"\nversion = \"0.27.2\"\n\n[[package]]\nname = \"typing-extensions\"\nversion = \"4.12.2\"\n").unwrap();
        fs::write(root.join("requirements-dev.txt"), "pytest==8.0.2\nblack\n").unwrap();

        let inventory = scan(&[root.clone()], &IgnoreSet::default());
        let api = inventory.projects.iter().find(|p| p.name == "api").expect("pyproject project");
        assert_eq!(api.ecosystem, Ecosystem::Pypi);
        assert_eq!(api.python_version.as_deref(), Some("3.11"));
        let dep = |name: &str| api.dependencies.iter().find(|d| d.name == name).unwrap();
        assert_eq!((dep("httpx").installed.as_deref(), dep("httpx").installed_from.as_deref()), (Some("0.27.2"), Some("uv.lock")));
        assert_eq!(dep("Typing_Extensions").installed.as_deref(), Some("4.12.2"), "names match the lockfile after normalizing");
        assert_eq!(dep("shared").status, Status::Local);
        assert_eq!(dep("mkdocs").kind, DepKind::Dev);

        let dev = inventory.projects.iter().find(|p| p.name.ends_with("(requirements-dev.txt)")).expect("requirements project");
        let pytest = dev.dependencies.iter().find(|d| d.name == "pytest").unwrap();
        assert_eq!((pytest.installed.as_deref(), pytest.kind), (Some("8.0.2"), DepKind::Dev));
        assert_eq!(dev.dependencies.iter().find(|d| d.name == "black").unwrap().note.as_deref(), Some("no version given"));
    }

    #[test]
    fn ignore_rules_skip_folders_projects_and_patterns() {
        let root = std::env::temp_dir().join("mehen-test-ignore");
        let _ = fs::remove_dir_all(&root);
        let pkg = r#"{ "name": "x", "dependencies": { "left-pad": "^1.3.0" } }"#;
        for dir in ["keep", "keep/sub", "skip-repo", "temp/copy", "one-off"] {
            fs::create_dir_all(root.join(dir)).unwrap();
            fs::write(root.join(dir).join("package.json"), pkg).unwrap();
        }
        let rule = |id, kind, value: &str| IgnoreRule { id, kind, value: value.into(), note: None };
        let rules = [
            rule(1, IgnoreKind::Folder, &root.join("skip-repo").display().to_string()),
            rule(2, IgnoreKind::Pattern, "temp"),
            rule(3, IgnoreKind::Project, &root.join("one-off").join("package.json").display().to_string()),
        ];
        let roots = [root.clone()];
        let inventory = scan(&roots, &IgnoreSet::new(&rules, &roots));
        let mut dirs: Vec<String> = inventory.projects.iter().map(|p| dir_name(Path::new(&p.dir))).collect();
        dirs.sort();
        assert_eq!(dirs, ["keep", "sub"]);
        assert_eq!(inventory.ignored.len(), 3, "{:?}", inventory.ignored);

        let found = discover(&roots, &rules);
        assert_eq!(found.len(), 5);
        assert_eq!(found.iter().filter(|p| p.ignored_by.is_some()).count(), 3);
    }

    #[test]
    fn workflow_uses_lines() {
        let dir = std::env::temp_dir().join("mehen-test-workflow");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ci.yml");
        fs::write(
            &file,
            "jobs:\n  a:\n    steps:\n      - uses: actions/checkout@v4\n      - uses: docker/login-action@dbcb813823bdd20940b903addbd779551569679f # v3.4.0\n      - uses: ./local\n    uses: org/repo/.github/workflows/x.yml@main\n",
        )
        .unwrap();
        let deps = read_workflow(&file);
        assert_eq!(deps.len(), 3);
        assert_eq!(deps[0].name, "actions/checkout");
        assert_eq!(deps[0].requested, "v4");
        assert_eq!(deps[1].pinned_comment.as_deref(), Some("v3.4.0"));
        assert_eq!(deps[2].name, "org/repo");
        assert_eq!(deps[2].requested, "main");
    }
}
