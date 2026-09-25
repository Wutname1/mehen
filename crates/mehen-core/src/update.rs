//! Updating dependencies: a plan (exact file edits plus the commands to run)
//! that the user reviews, then `apply`, which writes the edits, runs the
//! install and verify commands, and restores every touched file if any fail.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use similar::TextDiff;
use yaml_rust2::YamlLoader;

use crate::model::{Dependency, Ecosystem, Project, Status};
use crate::registry::PackageInfo;
use crate::version::{Version, from_spec};

const STEP_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const OUTPUT_TAIL_LINES: usize = 80;

/// One package to move to a new version, as chosen in the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub name: String,
    /// The entry as currently written, to pick one when a package appears
    /// more than once (an action used at `@v3` and `@v4`).
    #[serde(default)]
    pub from: Option<String>,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedChange {
    pub name: String,
    pub from: String,
    pub to: String,
    /// The manifest entry before and after, e.g. `^18.2.0` -> `^19.1.0`.
    pub written_before: String,
    pub written_after: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEdit {
    pub path: String,
    pub before: String,
    pub after: String,
    /// Unified diff for review.
    pub diff: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepKind {
    /// Brings the lockfile in line with the edited manifest. Always runs.
    Install,
    /// Proves the project still builds. Optional.
    Verify,
    /// Runs the project's tests. Optional, together with `Verify`.
    Test,
}

impl StepKind {
    /// Build and test steps are the "checks" a user can switch off.
    pub fn is_check(self) -> bool {
        matches!(self, StepKind::Verify | StepKind::Test)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub kind: StepKind,
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePlan {
    pub project_id: String,
    pub project_name: String,
    pub ecosystem: Ecosystem,
    pub changes: Vec<PlannedChange>,
    pub edits: Vec<FileEdit>,
    pub steps: Vec<Step>,
    /// Files the steps may rewrite (lockfiles); restored along with the edits on failure.
    pub snapshots: Vec<String>,
    pub warnings: Vec<String>,
    /// The git repo the project lives in, if any.
    #[serde(default)]
    pub repo: Option<String>,
    /// Why the update cannot be committed (not a repo, or the files already
    /// have uncommitted changes that would be swept into the commit).
    #[serde(default)]
    pub commit_blocked: Option<String>,
    /// The checked-out branch, where a commit would land.
    #[serde(default)]
    pub branch: Option<String>,
    /// Set aside and install from scratch if the install finds a peer
    /// conflict (npm's ERESOLVE): a lockfile and `node_modules` left from
    /// before a framework moves as a group can refuse a set that fits.
    #[serde(default)]
    pub clean_retry: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepResult {
    pub label: String,
    pub kind: StepKind,
    pub ok: bool,
    pub output: String,
    pub ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOutcome {
    pub ok: bool,
    pub rolled_back: bool,
    pub error: Option<String>,
    pub steps: Vec<StepResult>,
    /// Short hash of the commit made for this update.
    #[serde(default)]
    pub committed: Option<String>,
    /// The update stayed applied but the commit failed (a hook, for example).
    #[serde(default)]
    pub commit_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEvent {
    pub index: usize,
    pub label: String,
    /// `running`, `ok`, `failed`, or `rolled-back` (index is meaningless then).
    pub state: String,
}

fn is_commit_sha(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Builds the plan without touching any file. `package_info` returns the
/// cached registry answer for a package (needed to turn an Actions tag into
/// its commit SHA).
pub fn plan(project: &Project, changes: &[Change], package_info: impl Fn(&str) -> Option<PackageInfo>) -> anyhow::Result<UpdatePlan> {
    if changes.is_empty() {
        bail!("Nothing selected to update");
    }
    let mut plan = UpdatePlan {
        project_id: project.id.clone(),
        project_name: project.name.clone(),
        ecosystem: project.ecosystem,
        changes: Vec::new(),
        edits: Vec::new(),
        steps: Vec::new(),
        snapshots: Vec::new(),
        warnings: Vec::new(),
        repo: project.repo.clone(),
        commit_blocked: None,
        branch: None,
        clean_retry: Vec::new(),
    };
    // One edit covers every entry with the same name and spelling (e.g. the
    // package listed in both dependencies and devDependencies).
    let mut seen = std::collections::HashSet::new();
    let deps: Vec<(&Dependency, &Change)> = changes
        .iter()
        .filter(|c| seen.insert((c.name.clone(), c.from.clone())))
        .map(|c| {
            project
                .dependencies
                .iter()
                .find(|d| d.name == c.name && d.status != Status::Local && c.from.as_ref().is_none_or(|f| *f == d.requested))
                .map(|d| (d, c))
                .ok_or_else(|| anyhow!("{} is not an updatable dependency of {}", c.name, project.name))
        })
        .collect::<anyhow::Result<_>>()?;

    let manifest = PathBuf::from(&project.manifest);
    let dir = PathBuf::from(&project.dir);
    let repo = project.repo.as_deref().map(PathBuf::from);

    match project.ecosystem {
        Ecosystem::Npm => plan_npm(&mut plan, &manifest, &dir, repo.as_deref(), &deps)?,
        Ecosystem::Cargo => plan_cargo(&mut plan, &manifest, &dir, repo.as_deref(), &deps)?,
        Ecosystem::Nuget => plan_nuget(&mut plan, &manifest, &dir, repo.as_deref(), &deps)?,
        Ecosystem::Go => plan_go(&mut plan, &manifest, &dir, &deps)?,
        Ecosystem::Pypi => plan_python(&mut plan, &manifest, &dir, &deps)?,
        Ecosystem::Pub => plan_pub(&mut plan, &manifest, &dir, &deps)?,
        Ecosystem::Packagist => plan_composer(&mut plan, &manifest, &dir, &deps)?,
        Ecosystem::RubyGems => plan_bundler(&mut plan, &manifest, &dir, &deps)?,
        Ecosystem::GithubActions => plan_actions(&mut plan, &dir, &deps, &package_info)?,
    }
    for edit in &mut plan.edits {
        edit.diff = unified_diff(&edit.path, &edit.before, &edit.after, repo.as_deref().unwrap_or(&dir));
    }
    plan.commit_blocked = match &repo {
        None => Some("not inside a git repository".into()),
        Some(r) => dirty_files(r, &touched_paths(&plan)).map(|dirty| format!("uncommitted changes in {dirty}")),
    };
    plan.branch = repo.as_deref().and_then(current_branch);
    Ok(plan)
}

/// The checked-out branch, or `detached HEAD`.
fn current_branch(repo: &Path) -> Option<String> {
    let out = git(repo).args(["rev-parse", "--abbrev-ref", "HEAD"]).output().ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match (out.status.success(), name.as_str()) {
        (false, _) | (true, "") => None,
        (true, "HEAD") => Some("detached HEAD".into()),
        (true, _) => Some(name),
    }
}

/// Swaps the built-in build and test steps for the user's own commands, run
/// in `cwd`. Each command is one program and its arguments, split on spaces.
pub fn use_check_commands(plan: &mut UpdatePlan, commands: &[String], cwd: &str) {
    plan.steps.retain(|s| !s.kind.is_check());
    for command in commands.iter().map(|c| c.trim()).filter(|c| !c.is_empty()) {
        let mut parts = command.split_whitespace();
        let program = parts.next().unwrap_or_default().to_string();
        plan.steps.push(Step { kind: StepKind::Test, label: command.to_string(), program, args: parts.map(str::to_string).collect(), cwd: cwd.to_string() });
    }
}

/// Every file an update writes: the edits plus lockfiles the steps rewrite.
pub(crate) fn touched_paths(plan: &UpdatePlan) -> Vec<String> {
    let mut paths: Vec<String> = plan.edits.iter().map(|e| e.path.clone()).chain(plan.snapshots.iter().cloned()).collect();
    paths.dedup();
    paths
}

fn git(repo: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(repo);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// Names of files that already differ from HEAD, so committing them would
/// also commit someone else's work. `None` when all are clean.
fn dirty_files(repo: &Path, paths: &[String]) -> Option<String> {
    let out = git(repo).args(["status", "--porcelain", "--"]).args(paths).output().ok()?;
    let dirty: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.starts_with("??"))
        .filter_map(|l| l.get(3..).map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()))
        .collect();
    (!dirty.is_empty()).then(|| dirty.join(", "))
}

/// Commits exactly the files this update touched, on the current branch.
/// Other staged changes are left alone; hooks run as usual; nothing is pushed.
fn commit(plan: &UpdatePlan, message: &str) -> Result<String, String> {
    let repo = PathBuf::from(plan.repo.as_ref().ok_or("not inside a git repository")?);
    commit_paths(&repo, &touched_paths(plan), &[message])
}

/// Commits `paths` (those that still exist) with one `-m` per message part:
/// the first is the subject, the rest become the body. Returns the short hash.
pub(crate) fn commit_paths(repo: &Path, paths: &[String], messages: &[&str]) -> Result<String, String> {
    let paths: Vec<&String> = paths.iter().filter(|p| Path::new(p).exists()).collect();
    let run = |cmd: &mut std::process::Command| -> Result<String, String> {
        let out = cmd.output().map_err(|e| format!("could not run git: {e}"))?;
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        if out.status.success() { Ok(text) } else { Err(tail(&text)) }
    };
    run(git(repo).args(["add", "--"]).args(&paths))?;
    let mut commit = git(repo);
    commit.arg("commit");
    for m in messages.iter().filter(|m| !m.trim().is_empty()) {
        commit.args(["-m", m]);
    }
    run(commit.arg("--").args(&paths))?;
    run(git(repo).args(["rev-parse", "--short", "HEAD"])).map(|h| h.trim().to_string())
}

fn unified_diff(path: &str, before: &str, after: &str, base: &Path) -> String {
    let shown = Path::new(path).strip_prefix(base).map(|p| p.display().to_string()).unwrap_or_else(|_| path.to_string());
    TextDiff::from_lines(before, after).unified_diff().context_radius(2).header(&shown, &shown).to_string()
}

fn read(path: &Path) -> anyhow::Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

/// Keeps the file's line endings when an editor library normalised them.
fn match_line_endings(before: &str, after: String) -> String {
    if before.contains("\r\n") && !after.contains("\r\n") { after.replace('\n', "\r\n") } else { after }
}

fn push_edit(plan: &mut UpdatePlan, path: &Path, before: String, after: String) {
    if before != after {
        let after = match_line_endings(&before, after);
        plan.edits.push(FileEdit { path: path.display().to_string(), before, after, diff: String::new() });
    }
}

fn push_step(plan: &mut UpdatePlan, kind: StepKind, label: &str, program: &str, args: &[&str], cwd: &Path) {
    plan.steps.push(Step {
        kind,
        label: label.to_string(),
        program: program.to_string(),
        args: args.iter().map(|a| a.to_string()).collect(),
        cwd: cwd.display().to_string(),
    });
}

/// Moving to its group's version, which is below the newest it has.
fn held_in_group(dep: &Dependency, to: &str) -> bool {
    let newest = dep.newest.as_deref().or(dep.latest.as_deref());
    dep.group_target.as_deref() == Some(to) && newest.is_some_and(|n| Version::parse(n) > Version::parse(to))
}

/// Keeps the operator and prefix the author used: `^18.2.0` -> `^19.1.0`.
fn keep_prefix(old: &str, to: &str) -> Option<String> {
    if old.contains(' ') || old.contains("||") || from_spec(old).is_none() {
        return None;
    }
    let prefix: String = old.chars().take_while(|c| !c.is_ascii_digit()).collect();
    Some(format!("{prefix}{to}"))
}

/// Replaces each capture's middle with `new`, keeping the text around it.
fn replace_middle(re: &Regex, text: &str, new: &str) -> Option<String> {
    re.is_match(text).then(|| re.replace_all(text, |c: &Captures| format!("{}{new}{}", &c[1], &c[2])).into_owned())
}

// ---------------------------------------------------------------- npm

fn plan_npm(plan: &mut UpdatePlan, manifest: &Path, dir: &Path, repo: Option<&Path>, deps: &[(&Dependency, &Change)]) -> anyhow::Result<()> {
    let before = read(manifest)?;
    let mut text = before.clone();
    for (dep, change) in deps {
        let mut new_spec = keep_prefix(&dep.requested, &change.to).ok_or_else(|| anyhow!("{}: cannot rewrite the range `{}` automatically", dep.name, dep.requested))?;
        // Moving with a group but kept below its newest (TypeScript at 6.0.x
        // for Angular 22): `^` would let npm take a newer minor the group
        // does not accept, so it stays on the patch line.
        if held_in_group(dep, &change.to) && new_spec.starts_with('^') {
            new_spec = format!("~{}", &new_spec[1..]);
        }
        let re = Regex::new(&format!(r#"("{}"\s*:\s*"){}(")"#, regex::escape(&dep.name), regex::escape(&dep.requested)))?;
        text = replace_middle(&re, &text, &new_spec).ok_or_else(|| anyhow!("{}: `{}` not found in package.json", dep.name, dep.requested))?;
        plan.changes.push(PlannedChange {
            name: dep.name.clone(),
            from: dep.current.clone().unwrap_or_else(|| dep.requested.clone()),
            to: change.to.clone(),
            written_before: dep.requested.clone(),
            written_after: new_spec,
        });
    }
    let json = serde_json::from_str::<serde_json::Value>(&before).ok();
    let scripts_build = json.as_ref().is_some_and(|j| j["scripts"]["build"].is_string());
    // `npm init` writes a test script that only fails; that is no test suite.
    let scripts_test = json.as_ref().and_then(|j| j["scripts"]["test"].as_str()).is_some_and(|s| !s.contains("no test specified"));
    push_edit(plan, manifest, before, text);

    match npm_manager(dir, repo) {
        Some((pm, lock_dir, lockfile)) => {
            plan.snapshots.push(lockfile.display().to_string());
            if pm == "npm" && deps.iter().any(|(d, c)| d.group_target.as_deref() == Some(c.to.as_str())) {
                plan.clean_retry = vec![lockfile.display().to_string(), lock_dir.join("node_modules").display().to_string()];
            }
            let mut args = vec!["install"];
            if pm == "pnpm" {
                args.push("--no-frozen-lockfile");
            }
            push_step(plan, StepKind::Install, &format!("{pm} install"), pm, &args, &lock_dir);
            if scripts_build {
                push_step(plan, StepKind::Verify, &format!("{pm} run build"), pm, &["run", "build"], dir);
            }
            if scripts_test {
                push_step(plan, StepKind::Test, &format!("{pm} run test"), pm, &["run", "test"], dir);
            }
        }
        None => plan.warnings.push("No lockfile found, so no install will run. Run your package manager's install yourself.".into()),
    }
    Ok(())
}

fn npm_manager(dir: &Path, repo: Option<&Path>) -> Option<(&'static str, PathBuf, PathBuf)> {
    for d in dir.ancestors() {
        for (file, pm) in [("pnpm-lock.yaml", "pnpm"), ("bun.lock", "bun"), ("bun.lockb", "bun"), ("yarn.lock", "yarn"), ("package-lock.json", "npm")] {
            let lock = d.join(file);
            if lock.is_file() {
                return Some((pm, d.to_path_buf(), lock));
            }
        }
        if Some(d) == repo {
            break;
        }
    }
    None
}

// ---------------------------------------------------------------- Cargo

/// Cargo specs are often written short (`1.0`); keep the author's precision.
fn cargo_spec(old: &str, to: &str) -> Option<String> {
    let prefix: String = old.chars().take_while(|c| !c.is_ascii_digit()).collect();
    let precision = from_spec(old)?.split('.').count();
    let target = Version::parse(to)?;
    let version = if precision >= target.parts.len() {
        to.to_string()
    } else {
        target.parts.iter().take(precision.max(1)).map(u64::to_string).collect::<Vec<_>>().join(".")
    };
    Some(format!("{prefix}{version}"))
}

fn set_str_keep_decor(value: &mut toml_edit::Value, new: &str) {
    let decor = value.decor().clone();
    *value = toml_edit::Value::from(new);
    *value.decor_mut() = decor;
}

/// Rewrites every entry for `name` in one dependency table; returns (before, after) specs.
fn cargo_visit(table: &mut toml_edit::Item, name: &str, to: &str, found: &mut Vec<(String, String)>) {
    let Some(table) = table.as_table_like_mut() else { return };
    for (key, item) in table.iter_mut() {
        let real = item.get("package").and_then(|p| p.as_str()).unwrap_or(key.get()).to_string();
        if real != name {
            continue;
        }
        let version_value: Option<&mut toml_edit::Value> = if item.as_value().is_some_and(|v| v.is_str()) {
            item.as_value_mut()
        } else if let Some(inline) = item.as_inline_table_mut() {
            inline.get_mut("version")
        } else if let Some(t) = item.as_table_mut() {
            t.get_mut("version").and_then(|v| v.as_value_mut())
        } else {
            None
        };
        if let Some(value) = version_value {
            if let Some(old) = value.as_str().map(str::to_string) {
                if let Some(new) = cargo_spec(&old, to) {
                    set_str_keep_decor(value, &new);
                    found.push((old, new));
                }
            }
        }
    }
}

fn plan_cargo(plan: &mut UpdatePlan, manifest: &Path, dir: &Path, repo: Option<&Path>, deps: &[(&Dependency, &Change)]) -> anyhow::Result<()> {
    const SECTIONS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];
    let before = read(manifest)?;
    let mut doc: toml_edit::DocumentMut = before.parse().context("parsing Cargo.toml")?;
    for (dep, change) in deps {
        let mut found = Vec::new();
        for section in SECTIONS {
            if let Some(item) = doc.get_mut(section) {
                cargo_visit(item, &dep.name, &change.to, &mut found);
            }
        }
        if let Some(item) = doc.get_mut("workspace").and_then(|w| w.get_mut("dependencies")) {
            cargo_visit(item, &dep.name, &change.to, &mut found);
        }
        if let Some(targets) = doc.get_mut("target").and_then(|t| t.as_table_like_mut()) {
            for (_, target) in targets.iter_mut() {
                for section in SECTIONS {
                    if let Some(item) = target.get_mut(section) {
                        cargo_visit(item, &dep.name, &change.to, &mut found);
                    }
                }
            }
        }
        let (old, new) = found.first().cloned().ok_or_else(|| anyhow!("{}: no version entry found in Cargo.toml", dep.name))?;
        plan.changes.push(PlannedChange {
            name: dep.name.clone(),
            from: dep.current.clone().unwrap_or(old.clone()),
            to: change.to.clone(),
            written_before: old,
            written_after: new,
        });
    }
    push_edit(plan, manifest, before, doc.to_string());

    if let Some(lock) = dir.ancestors().take_while(|d| repo.is_none_or(|r| d.starts_with(r))).map(|d| d.join("Cargo.lock")).find(|p| p.is_file()) {
        plan.snapshots.push(lock.display().to_string());
    }
    // A short requirement like `0.13` may already allow the target, leaving
    // the manifest unchanged, so the lockfile has to be moved explicitly.
    // `name@installed` stays unambiguous when the lock holds several versions.
    let mut args: Vec<String> = vec!["update".into()];
    for (dep, _) in deps {
        args.push("-p".into());
        args.push(match &dep.installed {
            Some(v) => format!("{}@{v}", dep.name),
            None => dep.name.clone(),
        });
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    push_step(plan, StepKind::Install, "cargo update (only the selected crates)", "cargo", &args, dir);
    push_step(plan, StepKind::Verify, "cargo check", "cargo", &["check", "--quiet"], dir);
    push_step(plan, StepKind::Test, "cargo test", "cargo", &["test", "--quiet"], dir);
    Ok(())
}

// ---------------------------------------------------------------- PHP

/// Rewrites each constraint in `composer.json`, then `composer update`
/// moves those packages (and what they need) in `composer.lock`.
fn plan_composer(plan: &mut UpdatePlan, manifest: &Path, dir: &Path, deps: &[(&Dependency, &Change)]) -> anyhow::Result<()> {
    let before = read(manifest)?;
    let mut text = before.clone();
    for (dep, change) in deps {
        let new = crate::php::rewrite(&dep.requested, &change.to);
        if new != dep.requested {
            let re = Regex::new(&format!(r#"("{}"\s*:\s*"){}(")"#, regex::escape(&dep.name), regex::escape(&dep.requested)))?;
            text = replace_middle(&re, &text, &new).ok_or_else(|| anyhow!("{}: `{}` not found in composer.json", dep.name, dep.requested))?;
        }
        plan.changes.push(PlannedChange { name: dep.name.clone(), from: dep.current.clone().unwrap_or_else(|| dep.requested.clone()), to: change.to.clone(), written_before: dep.requested.clone(), written_after: new });
    }
    let json = serde_json::from_str::<serde_json::Value>(&before).ok();
    let test_script = json.as_ref().is_some_and(|j| !j["scripts"]["test"].is_null());
    push_edit(plan, manifest, before, text);
    let lock = dir.join("composer.lock");
    if lock.is_file() {
        plan.snapshots.push(lock.display().to_string());
    }
    let mut args = vec!["update".to_string(), "--with-dependencies".to_string()];
    args.extend(deps.iter().map(|(d, _)| d.name.clone()));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    push_step(plan, StepKind::Install, "composer update", "composer", &args, dir);
    if test_script {
        push_step(plan, StepKind::Test, "composer test", "composer", &["run-script", "test"], dir);
    } else if ["phpunit.xml", "phpunit.xml.dist"].iter().any(|f| dir.join(f).is_file()) {
        push_step(plan, StepKind::Test, "phpunit", "composer", &["exec", "phpunit"], dir);
    }
    Ok(())
}

// ---------------------------------------------------------------- Ruby

/// Replaces the version strings at the start of `name`'s `gem` line, keeping
/// its quotes and options. A gem with no version is left as written.
fn gemfile_set(text: &str, name: &str, new: &str) -> Option<String> {
    let re = Regex::new(&format!(r#"(?m)^([ \t]*gem[ \t]+(['"]){}['"])((?:[ \t]*,[ \t]*['"][^'"\n]*['"])+)"#, regex::escape(name))).ok()?;
    let caps = re.captures(text)?;
    let quote = &caps[2];
    let strings = new.split(", ").map(|r| format!("{quote}{r}{quote}")).collect::<Vec<_>>().join(", ");
    let whole = caps.get(0)?;
    Some(format!("{}{}, {strings}{}", &text[..whole.start()], &caps[1], &text[whole.end()..]))
}

/// Rewrites each gem's requirement in the Gemfile, then `bundle update
/// --conservative` moves only those gems in `Gemfile.lock`.
fn plan_bundler(plan: &mut UpdatePlan, manifest: &Path, dir: &Path, deps: &[(&Dependency, &Change)]) -> anyhow::Result<()> {
    let before = read(manifest)?;
    let mut text = before.clone();
    for (dep, change) in deps {
        let new = crate::ruby::rewrite(&dep.requested, &change.to);
        if new != dep.requested && !dep.requested.is_empty() {
            text = gemfile_set(&text, &dep.name, &new).ok_or_else(|| anyhow!("{}: no version found on its gem line", dep.name))?;
        }
        plan.changes.push(PlannedChange { name: dep.name.clone(), from: dep.current.clone().unwrap_or_else(|| dep.requested.clone()), to: change.to.clone(), written_before: dep.requested.clone(), written_after: new });
    }
    push_edit(plan, manifest, before, text);
    let lock = dir.join("Gemfile.lock");
    if lock.is_file() {
        plan.snapshots.push(lock.display().to_string());
    }
    let mut args = vec!["update".to_string(), "--conservative".to_string()];
    args.extend(deps.iter().map(|(d, _)| d.name.clone()));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    push_step(plan, StepKind::Install, "bundle update", "bundle", &args, dir);
    if dir.join("spec").is_dir() {
        push_step(plan, StepKind::Test, "bundle exec rspec", "bundle", &["exec", "rspec"], dir);
    } else if dir.join("test").is_dir() && dir.join("Rakefile").is_file() {
        push_step(plan, StepKind::Test, "bundle exec rake test", "bundle", &["exec", "rake", "test"], dir);
    }
    Ok(())
}

// ---------------------------------------------------------------- Dart / Flutter

/// Rewrites each constraint in `pubspec.yaml` (an `any` constraint already
/// allows the target and stays), then `pub upgrade` moves `pubspec.lock`.
/// Flutter projects use `flutter`, plain Dart packages `dart`.
fn plan_pub(plan: &mut UpdatePlan, manifest: &Path, dir: &Path, deps: &[(&Dependency, &Change)]) -> anyhow::Result<()> {
    let before = read(manifest)?;
    let mut text = before.clone();
    for (dep, change) in deps {
        let (old, new) = match crate::dart::set_constraint(&text, &dep.name, &change.to) {
            Some((next, old, new)) => {
                text = next;
                (old, new)
            }
            None if dep.requested.trim().is_empty() || dep.requested.trim() == "any" => (dep.requested.clone(), dep.requested.clone()),
            None => bail!("{}: no version constraint found in pubspec.yaml", dep.name),
        };
        plan.changes.push(PlannedChange { name: dep.name.clone(), from: dep.current.clone().unwrap_or(old.clone()), to: change.to.clone(), written_before: old, written_after: new });
    }
    push_edit(plan, manifest, before, text.clone());
    let lock = dir.join("pubspec.lock");
    if lock.is_file() {
        plan.snapshots.push(lock.display().to_string());
    }
    let program = if crate::dart::parse(&text).map(|p| p.flutter).unwrap_or(false) { "flutter" } else { "dart" };
    let mut args = vec!["pub".to_string(), "upgrade".to_string()];
    args.extend(deps.iter().map(|(d, _)| d.name.clone()));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    push_step(plan, StepKind::Install, &format!("{program} pub upgrade"), program, &args, dir);
    push_step(plan, StepKind::Verify, &format!("{program} analyze"), program, &["analyze"], dir);
    if dir.join("test").is_dir() {
        push_step(plan, StepKind::Test, &format!("{program} test"), program, &["test"], dir);
    }
    Ok(())
}

// ---------------------------------------------------------------- Python

/// Rewrites the spec of each matching requirement string in a TOML array
/// (`[project] dependencies`, optional and dependency groups).
fn python_array(item: Option<&mut toml_edit::Item>, name: &str, from: &str, to: &str, found: &mut Vec<(String, String)>) {
    let Some(array) = item.and_then(|i| i.as_array_mut()) else { return };
    for value in array.iter_mut() {
        let Some(text) = value.as_str().map(str::to_string) else { continue };
        let Some(req) = crate::python::parse_requirement(&text) else { continue };
        if crate::python::normalize(&req.name) != crate::python::normalize(name) || req.spec != from || req.spec.is_empty() {
            continue;
        }
        let spec = crate::python::rewrite_spec(&req.spec, to);
        let at = text.find(&req.name).map(|i| i + req.name.len()).unwrap_or(0);
        let Some(pos) = text[at..].find(&req.spec).map(|i| i + at) else { continue };
        let rewritten = format!("{}{spec}{}", &text[..pos], &text[pos + req.spec.len()..]);
        let decor = value.decor().clone();
        *value = rewritten.into();
        *value.decor_mut() = decor;
        found.push((req.spec.clone(), spec));
    }
}

/// Same for every array in a table of groups (`optional-dependencies`, `dependency-groups`).
fn python_groups(item: Option<&mut toml_edit::Item>, name: &str, from: &str, to: &str, found: &mut Vec<(String, String)>) {
    let Some(groups) = item.and_then(|i| i.as_table_like_mut()) else { return };
    for (_, group) in groups.iter_mut() {
        python_array(Some(group), name, from, to, found);
    }
}

/// Rewrites `name = "spec"` or `name = { version = "spec" }` in a Poetry or Pipfile table.
fn python_table(item: Option<&mut toml_edit::Item>, name: &str, from: &str, to: &str, found: &mut Vec<(String, String)>) {
    let Some(table) = item.and_then(|i| i.as_table_like_mut()) else { return };
    for (key, entry) in table.iter_mut() {
        if crate::python::normalize(key.get()) != crate::python::normalize(name) {
            continue;
        }
        let slot = if entry.as_str().is_some() { Some(entry) } else { entry.as_table_like_mut().and_then(|t| t.get_mut("version")) };
        let Some(slot) = slot else { continue };
        let Some(old) = slot.as_str().map(str::to_string).filter(|s| s == from && !s.trim().is_empty() && s.trim() != "*") else { continue };
        let spec = crate::python::rewrite_spec(&old, to);
        if let Some(value) = slot.as_value_mut() {
            let decor = value.decor().clone();
            *value = spec.clone().into();
            *value.decor_mut() = decor;
            found.push((old, spec));
        }
    }
}

fn python_requirements_txt(text: &str, name: &str, from: &str, to: &str, found: &mut Vec<(String, String)>) -> String {
    let mut out = String::with_capacity(text.len());
    for raw in text.split_inclusive('\n') {
        let matched = crate::python::parse_requirement(raw)
            .filter(|r| !r.spec.is_empty() && r.spec == from && crate::python::normalize(&r.name) == crate::python::normalize(name))
            .and_then(|r| {
                let at = raw.find(&r.name).map(|i| i + r.name.len())?;
                let pos = raw[at..].find(&r.spec).map(|i| i + at)?;
                Some((r.spec, pos))
            });
        match matched {
            Some((old, pos)) => {
                let spec = crate::python::rewrite_spec(&old, to);
                out.push_str(&raw[..pos]);
                out.push_str(&spec);
                out.push_str(&raw[pos + old.len()..]);
                found.push((old, spec));
            }
            None => out.push_str(raw),
        }
    }
    out
}

/// Tests are run with pytest when the project has them.
fn python_has_tests(dir: &Path, manifest_text: &str) -> bool {
    ["tests", "test"].iter().any(|d| dir.join(d).is_dir())
        || ["pytest.ini", "conftest.py"].iter().any(|f| dir.join(f).is_file())
        || manifest_text.contains("[tool.pytest")
}

/// Edits the requirement specs, then relocks with the project's own tool
/// (uv, Poetry, PDM or Pipenv). A plain requirements file has no lockfile,
/// so only the file changes.
fn plan_python(plan: &mut UpdatePlan, manifest: &Path, dir: &Path, deps: &[(&Dependency, &Change)]) -> anyhow::Result<()> {
    let before = read(manifest)?;
    let file = manifest.file_name().and_then(|f| f.to_str()).unwrap_or_default().to_string();
    let mut text = before.clone();
    for (dep, change) in deps {
        let mut found = Vec::new();
        if file == "pyproject.toml" || file == "Pipfile" {
            let mut doc: toml_edit::DocumentMut = text.parse().with_context(|| format!("parsing {file}"))?;
            if file == "Pipfile" {
                for section in ["packages", "dev-packages"] {
                    python_table(doc.get_mut(section), &dep.name, &dep.requested, &change.to, &mut found);
                }
            } else {
                if let Some(project) = doc.get_mut("project") {
                    python_array(project.get_mut("dependencies"), &dep.name, &dep.requested, &change.to, &mut found);
                    python_groups(project.get_mut("optional-dependencies"), &dep.name, &dep.requested, &change.to, &mut found);
                }
                python_groups(doc.get_mut("dependency-groups"), &dep.name, &dep.requested, &change.to, &mut found);
                if let Some(tool) = doc.get_mut("tool") {
                    if let Some(pdm) = tool.get_mut("pdm") {
                        python_groups(pdm.get_mut("dev-dependencies"), &dep.name, &dep.requested, &change.to, &mut found);
                    }
                    if let Some(poetry) = tool.get_mut("poetry") {
                        python_table(poetry.get_mut("dependencies"), &dep.name, &dep.requested, &change.to, &mut found);
                        python_table(poetry.get_mut("dev-dependencies"), &dep.name, &dep.requested, &change.to, &mut found);
                        if let Some(groups) = poetry.get_mut("group").and_then(|g| g.as_table_like_mut()) {
                            for (_, group) in groups.iter_mut() {
                                python_table(group.get_mut("dependencies"), &dep.name, &dep.requested, &change.to, &mut found);
                            }
                        }
                    }
                }
            }
            text = doc.to_string();
        } else {
            text = python_requirements_txt(&text, &dep.name, &dep.requested, &change.to, &mut found);
        }
        let (old, new) = found.first().cloned().ok_or_else(|| anyhow!("{}: no versioned entry found in {file}", dep.name))?;
        plan.changes.push(PlannedChange { name: dep.name.clone(), from: dep.current.clone().unwrap_or(old.clone()), to: change.to.clone(), written_before: old, written_after: new });
    }
    push_edit(plan, manifest, before, text.clone());

    let Some((manager, lock)) = crate::python::Manager::for_manifest(manifest) else { return Ok(()) };
    plan.snapshots.push(lock.display().to_string());
    let names: Vec<String> = deps.iter().map(|(d, _)| d.name.clone()).collect();
    let program = manager.program();
    let mut args: Vec<String> = match manager {
        crate::python::Manager::Uv => vec!["lock".into()],
        crate::python::Manager::Poetry => vec!["update".into(), "--lock".into()],
        crate::python::Manager::Pdm => vec!["update".into(), "--no-sync".into()],
        crate::python::Manager::Pipenv => vec!["lock".into()],
    };
    match manager {
        crate::python::Manager::Uv => names.iter().for_each(|n| args.extend(["--upgrade-package".to_string(), n.clone()])),
        crate::python::Manager::Poetry | crate::python::Manager::Pdm => args.extend(names),
        crate::python::Manager::Pipenv => {}
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    push_step(plan, StepKind::Install, &format!("{program} {}", args[..args.len().min(2)].join(" ")), program, &args, dir);
    if python_has_tests(dir, &text) {
        push_step(plan, StepKind::Test, &format!("{program} run pytest"), program, &["run", "pytest", "-q"], dir);
    }
    Ok(())
}

// ---------------------------------------------------------------- Go

/// Moves each `require` line, then `go mod tidy` settles `go.sum` (and any
/// modules the new versions need in turn).
fn plan_go(plan: &mut UpdatePlan, manifest: &Path, dir: &Path, deps: &[(&Dependency, &Change)]) -> anyhow::Result<()> {
    let before = read(manifest)?;
    let mut text = before.clone();
    for (dep, change) in deps {
        let from = dep.current.clone().unwrap_or_else(|| dep.requested.clone());
        // The proxy lists versions with their `v`; a target typed without one still works.
        let to = if change.to.starts_with('v') { change.to.clone() } else { format!("v{}", change.to) };
        text = crate::golang::set_version(&text, &dep.name, &from, &to).ok_or_else(|| anyhow!("{}: no require line for {from} in go.mod", dep.name))?;
        plan.changes.push(PlannedChange { name: dep.name.clone(), from: from.clone(), to: to.clone(), written_before: from, written_after: to });
    }
    push_edit(plan, manifest, before, text);
    let sum = dir.join("go.sum");
    if sum.is_file() {
        plan.snapshots.push(sum.display().to_string());
    }
    push_step(plan, StepKind::Install, "go mod tidy", "go", &["mod", "tidy"], dir);
    push_step(plan, StepKind::Verify, "go build ./...", "go", &["build", "./..."], dir);
    push_step(plan, StepKind::Test, "go test ./...", "go", &["test", "./..."], dir);
    Ok(())
}

// ---------------------------------------------------------------- NuGet

fn nuget_patterns(name: &str, old: &str) -> anyhow::Result<Vec<Regex>> {
    let (n, o) = (regex::escape(name), regex::escape(old));
    let el = r"(?:PackageReference|PackageVersion)";
    Ok(vec![
        Regex::new(&format!(r#"(?is)(<{el}\b[^>]*?\b(?:Include|Update)\s*=\s*"{n}"[^>]*?\b(?:Version|VersionOverride)\s*=\s*"){o}(")"#))?,
        Regex::new(&format!(r#"(?is)(<{el}\b[^>]*?\b(?:Version|VersionOverride)\s*=\s*"){o}("[^>]*?\b(?:Include|Update)\s*=\s*"{n}")"#))?,
        Regex::new(&format!(r#"(?is)(<PackageReference\b[^>]*?\b(?:Include|Update)\s*=\s*"{n}"[^>]*>\s*<Version>\s*){o}(\s*</Version>)"#))?,
        Regex::new(&format!(r#"(?is)(<package\b[^>]*?\bid\s*=\s*"{n}"[^>]*?\bversion\s*=\s*"){o}(")"#))?,
        Regex::new(&format!(r#"(?is)(<package\b[^>]*?\bversion\s*=\s*"){o}("[^>]*?\bid\s*=\s*"{n}")"#))?,
    ])
}

fn plan_nuget(plan: &mut UpdatePlan, manifest: &Path, dir: &Path, repo: Option<&Path>, deps: &[(&Dependency, &Change)]) -> anyhow::Result<()> {
    let before = read(manifest)?;
    let mut text = before.clone();
    let is_packages_config = manifest.file_name().is_some_and(|n| n == "packages.config");
    let is_central = manifest.file_name().is_some_and(|n| n == "Directory.Packages.props");
    let mut hint_path_updates: Vec<(String, String, String)> = Vec::new();

    for (dep, change) in deps {
        if dep.requested.contains(['[', '(', '*']) {
            bail!("{}: version range `{}` must be edited by hand", dep.name, dep.requested);
        }
        let replaced = nuget_patterns(&dep.name, &dep.requested)?.iter().find_map(|re| replace_middle(re, &text, &change.to));
        text = replaced.ok_or_else(|| anyhow!("{}: `{}` not found in {}", dep.name, dep.requested, manifest.display()))?;
        if is_packages_config {
            hint_path_updates.push((dep.name.clone(), dep.requested.clone(), change.to.clone()));
        }
        plan.changes.push(PlannedChange {
            name: dep.name.clone(),
            from: dep.requested.clone(),
            to: change.to.clone(),
            written_before: dep.requested.clone(),
            written_after: change.to.clone(),
        });
    }
    push_edit(plan, manifest, before, text);

    // packages.config projects reference each assembly by a versioned folder
    // (`packages\Name.1.2.3\lib\...`), so the project file must move too.
    if is_packages_config {
        for project_file in project_files(dir) {
            let before = read(&project_file)?;
            let mut text = before.clone();
            for (name, old, new) in &hint_path_updates {
                let re = Regex::new(&format!(r"(?i)(packages[\\/]{}\.){}([\\/])", regex::escape(name), regex::escape(old)))?;
                if let Some(t) = replace_middle(&re, &text, new) {
                    text = t;
                }
            }
            push_edit(plan, &project_file, before, text);
        }
        plan.warnings.push("packages.config project: restore packages in Visual Studio (or `nuget restore`) after this update.".into());
        return Ok(());
    }
    if is_central {
        plan.warnings.push("Central package versions changed. Restore and build the solution to confirm every project accepts them.".into());
        return Ok(());
    }

    let lock = dir.join("packages.lock.json");
    if lock.is_file() {
        plan.snapshots.push(lock.display().to_string());
    }
    let project_arg = manifest.display().to_string();
    push_step(plan, StepKind::Install, "dotnet restore", "dotnet", &["restore", &project_arg], dir);
    push_step(plan, StepKind::Verify, "dotnet build", "dotnet", &["build", &project_arg, "--no-restore", "-v", "q"], dir);
    // Tests usually live in sibling projects, so run the solution's tests.
    if let Some(solution) = solution_file(dir, repo) {
        let name = solution.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let parent = solution.parent().unwrap_or(dir).to_path_buf();
        push_step(plan, StepKind::Test, &format!("dotnet test {name}"), "dotnet", &["test", &solution.display().to_string(), "-v", "q"], &parent);
    }
    Ok(())
}

/// The nearest `.sln` or `.slnx` from `dir` up to the repository root.
fn solution_file(dir: &Path, repo: Option<&Path>) -> Option<PathBuf> {
    for d in dir.ancestors() {
        if repo.is_some_and(|r| !d.starts_with(r)) {
            break;
        }
        let mut found: Vec<PathBuf> = std::fs::read_dir(d)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("sln") || e.eq_ignore_ascii_case("slnx")))
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        if let Some(first) = found.into_iter().next() {
            return Some(first);
        }
        if repo.is_none() {
            break;
        }
    }
    None
}

fn project_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| ["csproj", "vbproj", "fsproj"].iter().any(|x| e.eq_ignore_ascii_case(x))))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------- GitHub Actions

fn workflow_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join(".github").join("workflows"))
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "yml" || e == "yaml")).collect())
        .unwrap_or_default();
    files.extend(["action.yml", "action.yaml"].iter().map(|f| dir.join(f)).filter(|p| p.is_file()));
    files.sort();
    files
}

/// Keeps the pin style: `v4` -> `v7`, `v4.1.1` -> `v7.0.1`, a commit SHA ->
/// the new tag's commit with a `# v7.0.1` comment.
fn action_ref(old: &str, to: &str, info: Option<&PackageInfo>) -> anyhow::Result<(String, Option<String>)> {
    if is_commit_sha(old) {
        let info = info.ok_or_else(|| anyhow!("tag list not cached; run a check first"))?;
        let sha = info.tags.iter().find(|(tag, _)| tag == to).map(|(_, sha)| sha.clone()).ok_or_else(|| anyhow!("no commit found for tag {to}"))?;
        return Ok((sha, Some(to.to_string())));
    }
    if Version::parse(old).is_some_and(|v| v.parts.len() == 1) {
        let major = Version::parse(to).map(|v| v.part(0)).ok_or_else(|| anyhow!("cannot read version {to}"))?;
        let floating = if old.starts_with(['v', 'V']) { format!("v{major}") } else { major.to_string() };
        let exists = info.is_none_or(|i| i.tags.iter().any(|(t, _)| *t == floating));
        return Ok((if exists { floating } else { to.to_string() }, None));
    }
    Ok((to.to_string(), None))
}

fn plan_actions(plan: &mut UpdatePlan, dir: &Path, deps: &[(&Dependency, &Change)], package_info: &impl Fn(&str) -> Option<PackageInfo>) -> anyhow::Result<()> {
    let files = workflow_files(dir);
    let mut contents: Vec<(PathBuf, String, String)> = files.into_iter().map(|p| read(&p).map(|t| (p, t.clone(), t))).collect::<anyhow::Result<_>>()?;

    for (dep, change) in deps {
        let info = package_info(&dep.name);
        let (new_ref, comment) = action_ref(&dep.requested, &change.to, info.as_ref()).with_context(|| dep.name.clone())?;
        let re = Regex::new(&format!(
            r#"(?im)(uses:\s*["']?{}(?:/[^@\s"'#]*)?@){}(["']?)([ \t]*#[^\r\n]*)?"#,
            regex::escape(&dep.name),
            regex::escape(&dep.requested)
        ))?;
        let mut hit = false;
        for (_, _, text) in &mut contents {
            if re.is_match(text) {
                hit = true;
                *text = re
                    .replace_all(text, |c: &Captures| {
                        let tail = match &comment {
                            Some(tag) => format!(" # {tag}"),
                            None => c.get(3).map(|m| m.as_str().to_string()).unwrap_or_default(),
                        };
                        format!("{}{new_ref}{}{tail}", &c[1], &c[2])
                    })
                    .into_owned();
            }
        }
        if !hit {
            bail!("{}@{} not found in this repo's workflows", dep.name, dep.requested);
        }
        plan.changes.push(PlannedChange {
            name: dep.name.clone(),
            from: dep.current.clone().unwrap_or_else(|| dep.requested.clone()),
            to: change.to.clone(),
            written_before: dep.requested.clone(),
            written_after: comment.map(|c| format!("{} # {c}", &new_ref[..new_ref.len().min(12)])).unwrap_or(new_ref),
        });
    }

    for (path, before, after) in contents {
        if before != after {
            YamlLoader::load_from_str(&after).map_err(|e| anyhow!("{} would no longer be valid YAML: {e}", path.display()))?;
            push_edit(plan, &path, before, after);
        }
    }
    plan.warnings.push("Workflow changes are checked for valid YAML only; they run the next time the workflow triggers.".into());
    Ok(())
}

// ---------------------------------------------------------------- apply

pub(crate) struct Snapshot {
    path: PathBuf,
    contents: Option<Vec<u8>>,
}

/// Reads each file as it is now, so it can be put back exactly.
pub(crate) fn snapshot(paths: &[String]) -> Vec<Snapshot> {
    paths.iter().map(|p| Snapshot { contents: std::fs::read(p).ok(), path: PathBuf::from(p) }).collect()
}

pub(crate) fn restore(snapshots: &[Snapshot]) -> Result<(), String> {
    let mut failures = Vec::new();
    for s in snapshots {
        let result = match &s.contents {
            Some(bytes) => std::fs::write(&s.path, bytes),
            None if s.path.exists() => std::fs::remove_file(&s.path),
            None => Ok(()),
        };
        if let Err(e) = result {
            failures.push(format!("{}: {e}", s.path.display()));
        }
    }
    if failures.is_empty() { Ok(()) } else { Err(failures.join("; ")) }
}

pub(crate) fn tail(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    lines[lines.len().saturating_sub(OUTPUT_TAIL_LINES)..].join("\n")
}

/// The full path of a program found on PATH. On Windows, npm, pnpm, yarn and
/// bun are `.cmd` launchers; starting them by full path lets the standard
/// library pass arguments through cmd safely, where wrapping everything in
/// `cmd /C` would treat `&`, `>` or `|` in an argument or folder as shell syntax.
pub(crate) fn resolve_program(program: &str) -> PathBuf {
    let given = PathBuf::from(program);
    if !cfg!(windows) || given.extension().is_some() || given.components().count() > 1 {
        return given;
    }
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let dirs = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
    dirs.iter()
        .flat_map(|dir| exts.split(';').filter(|e| !e.is_empty()).map(move |ext| dir.join(format!("{program}{}", ext.to_lowercase()))))
        .find(|candidate| candidate.is_file())
        .unwrap_or(given)
}

/// Runs one step and returns whether it succeeded plus the end of its output.
pub async fn run_step(step: &Step) -> (bool, String) {
    let mut cmd = tokio::process::Command::new(resolve_program(&step.program));
    cmd.args(&step.args);
    cmd.current_dir(&step.cwd).kill_on_drop(true).stdin(std::process::Stdio::null());
    if step.kind == StepKind::Test {
        // Test runners that watch for changes by default run once under CI.
        cmd.env("CI", "true");
    }
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);

    match tokio::time::timeout(STEP_TIMEOUT, cmd.output()).await {
        Err(_) => (false, format!("Timed out after {} minutes", STEP_TIMEOUT.as_secs() / 60)),
        Ok(Err(e)) => (false, format!("Could not start `{}`: {e}", step.program)),
        Ok(Ok(out)) => {
            let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            (out.status.success(), text)
        }
    }
}

/// Writes the plan's edits and runs its steps. If the files changed since the
/// plan was made, nothing is written. If any step fails, every edited file and
/// lockfile is put back exactly as it was.
pub async fn apply(plan: &UpdatePlan, run_verify: bool, commit_message: Option<&str>, on_event: impl Fn(UpdateEvent)) -> UpdateOutcome {
    let mut outcome = UpdateOutcome { ok: false, rolled_back: false, error: None, steps: Vec::new(), committed: None, commit_error: None };

    for edit in &plan.edits {
        match std::fs::read_to_string(&edit.path) {
            Ok(current) if current == edit.before => {}
            Ok(_) => {
                outcome.error = Some(format!("{} changed since this update was reviewed. Review it again.", edit.path));
                return outcome;
            }
            Err(e) => {
                outcome.error = Some(format!("{}: {e}", edit.path));
                return outcome;
            }
        }
    }

    let mut paths: Vec<PathBuf> = plan.edits.iter().map(|e| PathBuf::from(&e.path)).collect();
    paths.extend(plan.snapshots.iter().map(PathBuf::from));
    paths.dedup();
    let snapshots: Vec<Snapshot> = paths.into_iter().map(|path| Snapshot { contents: std::fs::read(&path).ok(), path }).collect();

    let roll_back = |outcome: &mut UpdateOutcome, reason: String| {
        match restore(&snapshots) {
            Ok(()) => outcome.rolled_back = true,
            Err(e) => outcome.error = Some(format!("{reason}. Restoring files also failed: {e}")),
        }
        if outcome.error.is_none() {
            outcome.error = Some(reason);
        }
        on_event(UpdateEvent { index: 0, label: "Restored the original files".into(), state: "rolled-back".into() });
    };

    for edit in &plan.edits {
        if let Err(e) = std::fs::write(&edit.path, &edit.after) {
            roll_back(&mut outcome, format!("Could not write {}: {e}", edit.path));
            return outcome;
        }
    }

    for (index, step) in plan.steps.iter().enumerate() {
        if step.kind.is_check() && !run_verify {
            continue;
        }
        on_event(UpdateEvent { index, label: step.label.clone(), state: "running".into() });
        let started = Instant::now();
        let (ok, output) = run_step(step).await;
        let output = tail(&output);
        outcome.steps.push(StepResult { label: step.label.clone(), kind: step.kind, ok, output, ms: started.elapsed().as_millis() as u64 });
        on_event(UpdateEvent { index, label: step.label.clone(), state: if ok { "ok" } else { "failed" }.into() });
        if !ok {
            roll_back(&mut outcome, format!("`{}` failed", step.label));
            return outcome;
        }
    }

    outcome.ok = true;
    if let Some(message) = commit_message.filter(|m| !m.trim().is_empty()) {
        let index = plan.steps.len();
        on_event(UpdateEvent { index, label: "git commit".into(), state: "running".into() });
        match commit(plan, message) {
            Ok(hash) => {
                outcome.committed = Some(hash);
                on_event(UpdateEvent { index, label: "git commit".into(), state: "ok".into() });
            }
            Err(e) => {
                outcome.commit_error = Some(e);
                on_event(UpdateEvent { index, label: "git commit".into(), state: "failed".into() });
            }
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(name: &str, eco: Ecosystem, requested: &str, current: &str) -> Dependency {
        let mut d = Dependency::new(name, eco, crate::model::DepKind::Normal, requested);
        d.current = Some(current.into());
        d.status = Status::Major;
        d
    }

    fn project(dir: &Path, manifest: &str, eco: Ecosystem, deps: Vec<Dependency>) -> Project {
        Project {
            id: dir.join(manifest).display().to_string(),
            name: "test".into(),
            ecosystem: eco,
            dir: dir.display().to_string(),
            manifest: dir.join(manifest).display().to_string(),
            repo: Some(dir.display().to_string()),
            frameworks: Vec::new(),
            rust_version: None,
            node_version: None,
            node_engines: None,
            python_version: None,
            php_version: None,
            dependencies: deps,
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mehen-update-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_group_keeps_a_held_back_member_on_its_patch_line() {
        let dir = temp("npm-group");
        std::fs::write(dir.join("package.json"), "{\n  \"devDependencies\": {\n    \"typescript\": \"^5.1\",\n    \"@angular/compiler-cli\": \"~16.2.12\"\n  }\n}\n").unwrap();
        std::fs::write(dir.join("package-lock.json"), "{}").unwrap();
        let mut ts = dep("typescript", Ecosystem::Npm, "^5.1", "5.1.6");
        (ts.group_target, ts.newest) = (Some("6.0.3".into()), Some("7.0.2".into()));
        let mut cli = dep("@angular/compiler-cli", Ecosystem::Npm, "~16.2.12", "16.2.12");
        (cli.group_target, cli.latest) = (Some("22.2.0".into()), Some("22.2.0".into()));
        let p = project(&dir, "package.json", Ecosystem::Npm, vec![ts, cli]);
        let changes = [Change { from: None, name: "typescript".into(), to: "6.0.3".into() }, Change { from: None, name: "@angular/compiler-cli".into(), to: "22.2.0".into() }];
        let plan = plan(&p, &changes, |_| None).unwrap();
        assert!(plan.edits[0].after.contains("\"typescript\": \"~6.0.3\""), "npm must not take a newer 6.x: {}", plan.edits[0].after);
        assert!(plan.edits[0].after.contains("\"@angular/compiler-cli\": \"~22.2.0\""));
        assert_eq!(plan.clean_retry.len(), 2, "a group move may need a clean install");
    }

    #[test]
    fn npm_keeps_prefix_and_other_entries() {
        let dir = temp("npm");
        std::fs::write(dir.join("package.json"), "{\r\n  \"dependencies\": {\r\n    \"react\": \"^18.2.0\",\r\n    \"left-pad\": \"~1.0.0\"\r\n  }\r\n}\r\n").unwrap();
        let p = project(&dir, "package.json", Ecosystem::Npm, vec![dep("react", Ecosystem::Npm, "^18.2.0", "18.2.0")]);
        let plan = plan(&p, &[Change { from: None, name: "react".into(), to: "19.1.0".into() }], |_| None).unwrap();
        assert_eq!(plan.edits.len(), 1);
        assert!(plan.edits[0].after.contains("\"react\": \"^19.1.0\""));
        assert!(plan.edits[0].after.contains("\"left-pad\": \"~1.0.0\""));
        assert!(plan.edits[0].after.contains("\r\n"));
        assert_eq!(plan.changes[0].written_after, "^19.1.0");
    }

    #[test]
    fn python_requirements_txt_keeps_the_rest_of_the_line() {
        let dir = temp("py-req");
        std::fs::write(dir.join("requirements.txt"), "# web\r\nrequests[socks]>=2.28,<3  # http\r\nDjango==4.2.7 ; python_version >= \"3.10\"\r\nflask\r\n-r dev.txt\r\n").unwrap();
        let deps = vec![dep("requests", Ecosystem::Pypi, ">=2.28,<3", "2.31.0"), dep("django", Ecosystem::Pypi, "==4.2.7", "4.2.7")];
        let p = project(&dir, "requirements.txt", Ecosystem::Pypi, deps);
        let changes = [Change { from: None, name: "requests".into(), to: "3.1.0".into() }, Change { from: None, name: "django".into(), to: "5.1.4".into() }];
        let plan = plan(&p, &changes, |_| None).unwrap();
        let after = &plan.edits[0].after;
        assert!(after.contains("requests[socks]>=3.1.0,<4  # http\r\n"), "{after}");
        assert!(after.contains("Django==5.1.4 ; python_version >= \"3.10\"\r\n"), "{after}");
        assert!(after.contains("flask\r\n-r dev.txt"));
        assert!(plan.steps.is_empty(), "no lockfile, nothing to run");
    }

    #[test]
    fn python_pyproject_with_uv_relocks_and_tests() {
        let dir = temp("py-uv");
        std::fs::write(
            dir.join("pyproject.toml"),
            "[project]\nname = \"app\"\nrequires-python = \">=3.11\"\ndependencies = [\n  \"httpx>=0.27\",  # client\n  \"pydantic~=2.6\",\n]\n\n[dependency-groups]\ndev = [\"pytest>=8.0\"]\n\n[tool.pytest.ini_options]\naddopts = \"-q\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("uv.lock"), "version = 1\n").unwrap();
        let deps = vec![dep("httpx", Ecosystem::Pypi, ">=0.27", "0.27.2"), dep("pytest", Ecosystem::Pypi, ">=8.0", "8.0.2")];
        let p = project(&dir, "pyproject.toml", Ecosystem::Pypi, deps);
        let changes = [Change { from: None, name: "httpx".into(), to: "0.28.1".into() }, Change { from: None, name: "pytest".into(), to: "8.3.4".into() }];
        let plan = plan(&p, &changes, |_| None).unwrap();
        let after = &plan.edits[0].after;
        assert!(after.contains("  \"httpx>=0.28.1\",  # client\n"), "{after}");
        assert!(after.contains("  \"pydantic~=2.6\",\n"));
        assert!(after.contains("dev = [\"pytest>=8.3.4\"]"), "{after}");
        assert!(plan.snapshots.iter().any(|s| s.ends_with("uv.lock")));
        let steps: Vec<String> = plan.steps.iter().map(|s| format!("{} {}", s.program, s.args.join(" "))).collect();
        assert_eq!(steps, ["uv lock --upgrade-package httpx --upgrade-package pytest", "uv run pytest -q"]);
    }

    #[test]
    fn python_poetry_and_pipfile_tables() {
        let dir = temp("py-poetry");
        std::fs::write(
            dir.join("pyproject.toml"),
            "[tool.poetry]\nname = \"svc\"\n\n[tool.poetry.dependencies]\npython = \"^3.10\"\nfastapi = \"^0.110\"\nsqlalchemy = { version = \"^2.0\", extras = [\"asyncio\"] }\n\n[tool.poetry.group.dev.dependencies]\nruff = \"0.4.1\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("poetry.lock"), "").unwrap();
        let deps = vec![dep("fastapi", Ecosystem::Pypi, "^0.110", "0.110.3"), dep("sqlalchemy", Ecosystem::Pypi, "^2.0", "2.0.30"), dep("ruff", Ecosystem::Pypi, "0.4.1", "0.4.1")];
        let p = project(&dir, "pyproject.toml", Ecosystem::Pypi, deps);
        let changes = [
            Change { from: None, name: "fastapi".into(), to: "0.115.6".into() },
            Change { from: None, name: "sqlalchemy".into(), to: "2.0.36".into() },
            Change { from: None, name: "ruff".into(), to: "0.8.4".into() },
        ];
        let poetry = plan(&p, &changes, |_| None).unwrap();
        let after = &poetry.edits[0].after;
        assert!(after.contains("fastapi = \"^0.115\""), "{after}");
        assert!(after.contains("sqlalchemy = { version = \"^2.0\", extras = [\"asyncio\"] }"), "same line still allows it: {after}");
        assert!(after.contains("ruff = \"0.8.4\""), "{after}");
        assert!(after.contains("python = \"^3.10\""));
        assert_eq!(poetry.steps[0].args, ["update", "--lock", "fastapi", "sqlalchemy", "ruff"]);

        let pipenv = temp("py-pipenv");
        std::fs::write(pipenv.join("Pipfile"), "[packages]\nflask = \"==3.0.0\"\n\n[dev-packages]\npytest = {version = \">=8\"}\n").unwrap();
        std::fs::write(pipenv.join("Pipfile.lock"), "{}").unwrap();
        let deps = vec![dep("flask", Ecosystem::Pypi, "==3.0.0", "3.0.0"), dep("pytest", Ecosystem::Pypi, ">=8", "8.0.0")];
        let p = project(&pipenv, "Pipfile", Ecosystem::Pypi, deps);
        let changes = [Change { from: None, name: "flask".into(), to: "3.1.0".into() }, Change { from: None, name: "pytest".into(), to: "8.3.4".into() }];
        let pipfile = plan(&p, &changes, |_| None).unwrap();
        assert!(pipfile.edits[0].after.contains("flask = \"==3.1.0\""));
        assert!(pipfile.edits[0].after.contains("pytest = {version = \">=8.3.4\"}"), "{}", pipfile.edits[0].after);
        assert_eq!(pipfile.steps[0].program, "pipenv");
    }

    #[test]
    fn pub_rewrites_constraints_and_upgrades_with_flutter() {
        let dir = temp("pub");
        std::fs::write(dir.join("pubspec.yaml"), "name: app\n\ndependencies:\n  flutter:\n    sdk: flutter\n  http: ^1.1.0 # client\n  intl: any\n").unwrap();
        std::fs::write(dir.join("pubspec.lock"), "packages: {}\n").unwrap();
        std::fs::create_dir_all(dir.join("test")).unwrap();
        let deps = vec![dep("http", Ecosystem::Pub, "^1.1.0", "1.2.2"), dep("intl", Ecosystem::Pub, "any", "0.19.0")];
        let p = project(&dir, "pubspec.yaml", Ecosystem::Pub, deps);
        let changes = [Change { from: None, name: "http".into(), to: "1.4.0".into() }, Change { from: None, name: "intl".into(), to: "0.20.2".into() }];
        let plan = plan(&p, &changes, |_| None).unwrap();
        assert!(plan.edits[0].after.contains("  http: ^1.4.0 # client\n"));
        assert!(plan.edits[0].after.contains("  intl: any\n"), "any already allows it");
        assert_eq!(plan.changes[1].written_after, "any");
        let steps: Vec<String> = plan.steps.iter().map(|s| format!("{} {}", s.program, s.args.join(" "))).collect();
        assert_eq!(steps, ["flutter pub upgrade http intl", "flutter analyze", "flutter test"]);
    }

    #[test]
    fn composer_rewrites_constraints_and_updates() {
        let dir = temp("composer");
        std::fs::write(
            dir.join("composer.json"),
            "{\n    \"require\": {\n        \"php\": \"^8.1\",\n        \"monolog/monolog\": \"^2.9\",\n        \"guzzlehttp/guzzle\": \"^7.2 || ^8.0\"\n    },\n    \"scripts\": {\"test\": \"phpunit\"}\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("composer.lock"), "{}").unwrap();
        let deps = vec![dep("monolog/monolog", Ecosystem::Packagist, "^2.9", "2.9.3"), dep("guzzlehttp/guzzle", Ecosystem::Packagist, "^7.2 || ^8.0", "7.9.2")];
        let p = project(&dir, "composer.json", Ecosystem::Packagist, deps);
        let changes = [Change { from: None, name: "monolog/monolog".into(), to: "3.8.1".into() }, Change { from: None, name: "guzzlehttp/guzzle".into(), to: "9.0.1".into() }];
        let plan = plan(&p, &changes, |_| None).unwrap();
        let after = &plan.edits[0].after;
        assert!(after.contains("\"monolog/monolog\": \"^3.8\""), "{after}");
        assert!(after.contains("\"guzzlehttp/guzzle\": \"^9.0\""), "{after}");
        assert!(after.contains("\"php\": \"^8.1\""));
        let steps: Vec<String> = plan.steps.iter().map(|s| format!("{} {}", s.program, s.args.join(" "))).collect();
        assert_eq!(steps, ["composer update --with-dependencies monolog/monolog guzzlehttp/guzzle", "composer run-script test"]);
    }

    #[test]
    fn bundler_rewrites_gem_lines_and_updates_conservatively() {
        let dir = temp("bundler");
        std::fs::write(dir.join("Gemfile"), "source \"https://rubygems.org\"\n\ngem \"rails\", \"~> 7.1\", \">= 7.1.3\" # app\ngem 'puma', '>= 5.0', require: false\ngem 'bootsnap'\n").unwrap();
        std::fs::write(dir.join("Gemfile.lock"), "GEM\n").unwrap();
        std::fs::create_dir_all(dir.join("spec")).unwrap();
        let deps = vec![dep("rails", Ecosystem::RubyGems, "~> 7.1, >= 7.1.3", "7.1.3"), dep("puma", Ecosystem::RubyGems, ">= 5.0", "6.4.2"), dep("bootsnap", Ecosystem::RubyGems, "", "1.18.3")];
        let p = project(&dir, "Gemfile", Ecosystem::RubyGems, deps);
        let changes = [
            Change { from: None, name: "rails".into(), to: "8.0.1".into() },
            Change { from: None, name: "puma".into(), to: "6.5.0".into() },
            Change { from: None, name: "bootsnap".into(), to: "1.18.4".into() },
        ];
        let plan = plan(&p, &changes, |_| None).unwrap();
        let after = &plan.edits[0].after;
        assert!(after.contains("gem \"rails\", \"~> 8.0\", \">= 8.0.1\" # app\n"), "{after}");
        assert!(after.contains("gem 'puma', '>= 6.5.0', require: false\n"), "{after}");
        assert!(after.contains("gem 'bootsnap'\n"), "no version, only the lockfile moves");
        let steps: Vec<String> = plan.steps.iter().map(|s| format!("{} {}", s.program, s.args.join(" "))).collect();
        assert_eq!(steps, ["bundle update --conservative rails puma bootsnap", "bundle exec rspec"]);
    }

    #[test]
    fn go_moves_the_require_line_and_tidies() {
        let dir = temp("go");
        std::fs::write(dir.join("go.mod"), "module example.com/app\n\ngo 1.26\n\nrequire (\n\tgithub.com/gin-gonic/gin v1.10.0\n\tgithub.com/google/uuid v1.6.0\n)\n").unwrap();
        std::fs::write(dir.join("go.sum"), "").unwrap();
        let p = project(&dir, "go.mod", Ecosystem::Go, vec![dep("github.com/gin-gonic/gin", Ecosystem::Go, "v1.10.0", "v1.10.0")]);
        let plan = plan(&p, &[Change { from: None, name: "github.com/gin-gonic/gin".into(), to: "v1.12.0".into() }], |_| None).unwrap();
        assert!(plan.edits[0].after.contains("\tgithub.com/gin-gonic/gin v1.12.0\n"));
        assert!(plan.edits[0].after.contains("\tgithub.com/google/uuid v1.6.0\n"));
        assert!(plan.snapshots.iter().any(|s| s.ends_with("go.sum")), "go.sum is put back if a step fails");
        let steps: Vec<String> = plan.steps.iter().map(|s| format!("{} {}", s.program, s.args.join(" "))).collect();
        assert_eq!(steps, ["go mod tidy", "go build ./...", "go test ./..."]);
    }

    #[test]
    fn cargo_keeps_precision_comments_and_tables() {
        let dir = temp("cargo");
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"x\"\n\n[dependencies]\nserde = { version = \"1.0\", features = [\"derive\"] } # keep me\ntokio = \"1.40.0\"\n\n[dev-dependencies.regex]\nversion = \"1\"\n",
        )
        .unwrap();
        let deps = vec![dep("serde", Ecosystem::Cargo, "1.0", "1.0.100"), dep("tokio", Ecosystem::Cargo, "1.40.0", "1.40.0"), dep("regex", Ecosystem::Cargo, "1", "1.9.0")];
        let p = project(&dir, "Cargo.toml", Ecosystem::Cargo, deps);
        let changes = [
            Change { from: None, name: "serde".into(), to: "2.1.5".into() },
            Change { from: None, name: "tokio".into(), to: "1.53.1".into() },
            Change { from: None, name: "regex".into(), to: "2.0.0".into() },
        ];
        let plan = plan(&p, &changes, |_| None).unwrap();
        let after = &plan.edits[0].after;
        assert!(after.contains(r#"serde = { version = "2.1", features = ["derive"] } # keep me"#), "{after}");
        assert!(after.contains(r#"tokio = "1.53.1""#), "{after}");
        assert!(after.contains("[dev-dependencies.regex]\nversion = \"2\""), "{after}");
    }

    #[test]
    fn nuget_handles_attribute_order_and_child_element() {
        let dir = temp("nuget");
        std::fs::write(
            dir.join("app.csproj"),
            "<Project>\n  <ItemGroup>\n    <PackageReference Include=\"Newtonsoft.Json\" Version=\"12.0.1\" />\n    <PackageReference Version=\"1.0.0\" Include=\"Serilog\" />\n    <PackageReference Include=\"Dapper\">\n      <Version>2.0.0</Version>\n    </PackageReference>\n  </ItemGroup>\n</Project>\n",
        )
        .unwrap();
        let deps = vec![dep("Newtonsoft.Json", Ecosystem::Nuget, "12.0.1", "12.0.1"), dep("Serilog", Ecosystem::Nuget, "1.0.0", "1.0.0"), dep("Dapper", Ecosystem::Nuget, "2.0.0", "2.0.0")];
        let p = project(&dir, "app.csproj", Ecosystem::Nuget, deps);
        let changes = [
            Change { from: None, name: "Newtonsoft.Json".into(), to: "13.0.3".into() },
            Change { from: None, name: "Serilog".into(), to: "4.0.0".into() },
            Change { from: None, name: "Dapper".into(), to: "2.1.35".into() },
        ];
        let plan = plan(&p, &changes, |_| None).unwrap();
        let after = &plan.edits[0].after;
        assert!(after.contains("Include=\"Newtonsoft.Json\" Version=\"13.0.3\""));
        assert!(after.contains("Version=\"4.0.0\" Include=\"Serilog\""));
        assert!(after.contains("<Version>2.1.35</Version>"));
        assert_eq!(plan.steps.len(), 2);
    }

    #[test]
    fn actions_keep_pin_style() {
        let dir = temp("actions");
        let wf = dir.join(".github").join("workflows");
        std::fs::create_dir_all(&wf).unwrap();
        std::fs::write(
            wf.join("ci.yml"),
            "jobs:\n  a:\n    steps:\n      - uses: actions/checkout@v4\n      - uses: docker/login-action@1111111111111111111111111111111111111111 # v3.0.0\n",
        )
        .unwrap();
        let mut docker = dep("docker/login-action", Ecosystem::GithubActions, "1111111111111111111111111111111111111111", "v3.0.0");
        docker.kind = crate::model::DepKind::Action;
        let deps = vec![dep("actions/checkout", Ecosystem::GithubActions, "v4", "v4"), docker];
        let mut p = project(&dir, ".github", Ecosystem::GithubActions, deps);
        p.manifest = dir.join(".github").display().to_string();
        let info = |name: &str| -> Option<PackageInfo> {
            Some(match name {
                "actions/checkout" => PackageInfo { latest: Some("v7.0.1".into()), versions: vec![], requirements: vec![], tags: vec![("v7".into(), "a".repeat(40)), ("v7.0.1".into(), "a".repeat(40))] },
                _ => PackageInfo { latest: Some("v4.1.0".into()), versions: vec![], requirements: vec![], tags: vec![("v4.1.0".into(), "2".repeat(40))] },
            })
        };
        let changes = [Change { from: None, name: "actions/checkout".into(), to: "v7.0.1".into() }, Change { from: None, name: "docker/login-action".into(), to: "v4.1.0".into() }];
        let plan = plan(&p, &changes, info).unwrap();
        let after = &plan.edits[0].after;
        assert!(after.contains("uses: actions/checkout@v7\n"), "{after}");
        assert!(after.contains(&format!("uses: docker/login-action@{} # v4.1.0", "2".repeat(40))), "{after}");
    }

    #[tokio::test]
    async fn commits_only_the_touched_files() {
        let dir = temp("commit");
        let git = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "test"]);
        std::fs::write(dir.join("package.json"), "{ \"dependencies\": { \"react\": \"^18.2.0\" } }").unwrap();
        std::fs::write(dir.join("other.txt"), "one").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);
        // Someone else's staged work must stay out of the update commit.
        std::fs::write(dir.join("other.txt"), "two").unwrap();
        git(&["add", "other.txt"]);

        let p = project(&dir, "package.json", Ecosystem::Npm, vec![dep("react", Ecosystem::Npm, "^18.2.0", "18.2.0")]);
        let mut first = plan(&p, &[Change { from: None, name: "react".into(), to: "19.1.0".into() }], |_| None).unwrap();
        assert!(first.commit_blocked.is_none(), "{:?}", first.commit_blocked);
        first.steps.clear();
        let outcome = apply(&first, false, Some("chore(deps): update react to 19.1.0"), |_| {}).await;
        assert!(outcome.ok && outcome.committed.is_some(), "{:?}", outcome.commit_error);

        let files = String::from_utf8(git(&["show", "--name-only", "--format=", "HEAD"]).stdout).unwrap();
        assert_eq!(files.trim(), "package.json");
        let staged = String::from_utf8(git(&["diff", "--cached", "--name-only"]).stdout).unwrap();
        assert_eq!(staged.trim(), "other.txt");

        // A manifest with uncommitted edits cannot be committed by Mehen.
        std::fs::write(dir.join("package.json"), "{ \"dependencies\": { \"react\": \"^19.1.0\" }, \"x\": 1 }").unwrap();
        let p = project(&dir, "package.json", Ecosystem::Npm, vec![dep("react", Ecosystem::Npm, "^19.1.0", "19.1.0")]);
        let second = plan(&p, &[Change { from: None, name: "react".into(), to: "19.2.0".into() }], |_| None).unwrap();
        assert!(second.commit_blocked.as_deref().is_some_and(|r| r.contains("package.json")), "{:?}", second.commit_blocked);
    }

    #[tokio::test]
    async fn failed_step_restores_files() {
        let dir = temp("rollback");
        let manifest = dir.join("package.json");
        std::fs::write(&manifest, "{ \"dependencies\": { \"react\": \"^18.2.0\" } }").unwrap();
        std::fs::write(dir.join("package-lock.json"), "{}").unwrap();
        let p = project(&dir, "package.json", Ecosystem::Npm, vec![dep("react", Ecosystem::Npm, "^18.2.0", "18.2.0")]);
        let mut plan = plan(&p, &[Change { from: None, name: "react".into(), to: "19.1.0".into() }], |_| None).unwrap();
        // Swap the install for a command that always fails and edits the lockfile first.
        plan.steps = vec![Step {
            kind: StepKind::Install,
            label: "fail".into(),
            program: if cfg!(windows) { "cmd".into() } else { "sh".into() },
            args: if cfg!(windows) { vec!["/C".into(), "echo broken> package-lock.json && exit 1".into()] } else { vec!["-c".into(), "echo broken > package-lock.json; exit 1".into()] },
            cwd: dir.display().to_string(),
        }];
        let outcome = apply(&plan, true, None, |_| {}).await;
        assert!(!outcome.ok);
        assert!(outcome.rolled_back, "{:?}", outcome.error);
        assert_eq!(std::fs::read_to_string(&manifest).unwrap(), "{ \"dependencies\": { \"react\": \"^18.2.0\" } }");
        assert_eq!(std::fs::read_to_string(dir.join("package-lock.json")).unwrap(), "{}");
    }
}
