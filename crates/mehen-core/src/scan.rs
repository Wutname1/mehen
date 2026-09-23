//! Walks a folder tree and reads every manifest it understands.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use ignore::WalkBuilder;
use regex::Regex;

use crate::lockfiles::LockfileCache;
use crate::model::{DepKind, Dependency, Ecosystem, Inventory, Project, Status};
use crate::version::{Version, from_spec};

const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", "bin", "obj", ".git", "dist", "build", "out", ".next", ".nuxt", ".turbo", ".svelte-kit", ".venv", "venv",
    ".vs", ".idea", "coverage", ".gradle", "Pods", ".pnpm-store", ".yarn",
];

static USES_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^[\s-]*uses:\s*["']?([^"'\s#]+)["']?[ \t]*(?:#[ \t]*(\S+))?"#).unwrap());

pub fn scan(root: &Path) -> Inventory {
    let start = Instant::now();
    let skipped = Arc::new(Mutex::new(Vec::new()));
    let skipped_in_filter = Arc::clone(&skipped);

    let walker = WalkBuilder::new(root)
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
            if entry.depth() > 0 && is_linked_worktree(entry.path()) {
                skipped_in_filter.lock().unwrap().push(entry.path().display().to_string());
                return false;
            }
            true
        })
        .build();

    let mut scanner = Scanner { root: root.to_path_buf(), ..Default::default() };
    let mut workflows: BTreeMap<PathBuf, Vec<(Dependency, PathBuf)>> = BTreeMap::new();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();

        let result = match file_name.as_ref() {
            "package.json" => scanner.package_json(path),
            "Cargo.toml" => scanner.cargo_toml(path),
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
        scanner.push_workflow_project(&dir, deps);
    }

    scanner.projects.sort_by(|a, b| a.dir.to_lowercase().cmp(&b.dir.to_lowercase()).then(a.name.cmp(&b.name)));
    let skipped_worktrees = std::mem::take(&mut *skipped.lock().unwrap());

    Inventory {
        root: root.display().to_string(),
        projects: scanner.projects,
        vulnerabilities: Vec::new(),
        skipped_worktrees,
        warnings: scanner.warnings,
        scan_ms: start.elapsed().as_millis() as u64,
        check_ms: None,
        check_stats: None,
    }
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
    root: PathBuf,
    projects: Vec<Project>,
    warnings: Vec<String>,
    cargo_locks: HashMap<PathBuf, HashMap<String, Vec<String>>>,
    npm_locks: LockfileCache,
}

impl Scanner {
    fn push(&mut self, manifest: &Path, name: String, ecosystem: Ecosystem, frameworks: Vec<String>, dependencies: Vec<Dependency>) {
        if dependencies.is_empty() {
            return;
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
            dependencies,
        });
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
        self.push(path, name, Ecosystem::Npm, Vec::new(), deps);
        Ok(())
    }

    fn npm_installed(&mut self, dir: &Path, name: &str) -> Option<(String, &'static str)> {
        let from_node_modules = dir.ancestors().take_while(|d| d.starts_with(&self.root)).find_map(|d| {
            let manifest = d.join("node_modules").join(name).join("package.json");
            let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(manifest).ok()?).ok()?;
            json["version"].as_str().map(str::to_string)
        });
        match from_node_modules {
            Some(v) => Some((v, "node_modules")),
            None => self.npm_locks.npm_version(&self.root, dir, name),
        }
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
        self.push(path, name, Ecosystem::Cargo, Vec::new(), deps);
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
        let lock_path = dir.ancestors().take_while(|d| d.starts_with(&self.root)).map(|d| d.join("Cargo.lock")).find(|p| p.is_file())?;
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
