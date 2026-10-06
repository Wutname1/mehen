//! Fixing a project's own code after an update breaks its build or tests,
//! with an AI coding tool the user already has (Claude Code or Codex). The
//! update goes in, the checks run, and while one fails the tool is shown the
//! failure and may edit files; Mehen runs the checks again itself. When they
//! never pass, every file the tool or the update touched is put back.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::cancel::Cancel;
use crate::update::{self, Step, StepKind, StepResult, UpdatePlan};

#[derive(Debug, Clone)]
pub struct FixOptions {
    /// The tool's name for messages: "Claude Code".
    pub tool: String,
    pub build: bool,
    pub test: bool,
    pub commit: bool,
    pub push: bool,
    /// How many times the tool is asked before giving up.
    pub rounds: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FixEvent {
    Step { label: String, state: String },
    /// The tool is being asked, for the `round`th time of at most `of`.
    Asking { round: usize, of: usize },
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FixOutcome {
    pub ok: bool,
    /// Times the tool was asked; 0 when the update needed no fix after all.
    pub rounds: usize,
    /// Files outside the update's own that changed, relative to the repository.
    pub changed: Vec<String>,
    /// What the tool said it did, from its last answer.
    pub summary: String,
    pub error: Option<String>,
    /// The last run of each step.
    pub steps: Vec<StepResult>,
    pub rolled_back: bool,
    pub committed: Option<String>,
    pub commit_error: Option<String>,
    pub commit_skipped: Option<String>,
    pub pushed: bool,
    pub push_error: Option<String>,
}

/// Files that differ from HEAD (and untracked ones), by path relative to the
/// repository, with whether git tracks them.
fn changed_files(repo: &Path) -> BTreeMap<String, bool> {
    let Ok(out) = update::git(repo).args(["status", "--porcelain", "-uall", "--no-renames"]).output() else { return BTreeMap::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let path = l.get(3..)?.trim().trim_matches('"').to_string();
            Some((path, !l.starts_with("??")))
        })
        .collect()
}

/// Every step of these plans once, installs first.
fn steps_of(plans: &[UpdatePlan], build: bool, test: bool) -> (Vec<Step>, Vec<Step>) {
    let mut seen = HashSet::new();
    let mut installs = Vec::new();
    let mut checks = Vec::new();
    for kind in [StepKind::Install, StepKind::Verify, StepKind::Test] {
        if kind == StepKind::Verify && !build || kind == StepKind::Test && !test {
            continue;
        }
        for step in plans.iter().flat_map(|p| &p.steps).filter(|s| s.kind == kind) {
            if seen.insert(update::step_key(step)) {
                if kind == StepKind::Install { installs.push(step.clone()) } else { checks.push(step.clone()) }
            }
        }
    }
    (installs, checks)
}

/// The end of a long output, which is where compilers and test runners put
/// what failed.
fn last_chars(output: &str, max: usize) -> &str {
    if output.len() <= max {
        return output;
    }
    let mut at = output.len() - max;
    while !output.is_char_boundary(at) {
        at += 1;
    }
    &output[at..]
}

/// What the tool is told: what moved, what fails, and what it may not do.
fn prompt(plans: &[UpdatePlan], failing: &[(String, String)], protected: &[String], checks: &[Step], earlier: &[String]) -> String {
    let mut moved: Vec<String> = plans.iter().flat_map(|p| &p.changes).map(|c| format!("- {} {} to {}", c.name, c.from, c.to)).collect();
    moved.dedup();
    let commands: Vec<String> = checks.iter().map(|s| format!("`{}`", std::iter::once(s.program.as_str()).chain(s.args.iter().map(String::as_str)).collect::<Vec<_>>().join(" "))).collect();
    let mut text = format!(
        "These dependencies of this project were just updated:\n{}\n\nSince then, {} fails. Change the project's own code so it works with the new versions.\n\n\
         Rules:\n\
         - Do not change dependency versions, manifests or lockfiles ({}).\n\
         - Do not delete, skip or weaken tests to make them pass.\n\
         - Keep changes as small as you can, in the style of the code around them.\n\
         - You cannot run commands. When you finish, {} run again and you will be told what still fails.\n",
        moved.join("\n"),
        failing.iter().map(|(label, _)| format!("`{label}`")).collect::<Vec<_>>().join(" and "),
        protected.join(", "),
        commands.join(" and "),
    );
    if !earlier.is_empty() {
        text.push_str(&format!("\nAn earlier attempt already changed {}, and the failure below is what remains.\n", earlier.join(", ")));
    }
    for (label, output) in failing {
        text.push_str(&format!("\nOutput of `{label}`:\n```\n{}\n```\n", last_chars(output.trim(), 12_000)));
    }
    text
}

/// Applies `plans` (one repository's update), then asks the tool to make the
/// checks pass, up to `options.rounds` times. `agent(prompt, cancel)` runs the
/// tool in the repository and returns what it said it did.
pub async fn fix<R, RF, A, AF, E>(plans: Vec<UpdatePlan>, options: FixOptions, cancel: &Cancel, run_step: R, agent: A, on_event: E) -> FixOutcome
where
    R: Fn(Step, Cancel) -> RF,
    RF: Future<Output = (bool, String)>,
    A: Fn(String, Cancel) -> AF,
    AF: Future<Output = Result<String, String>>,
    E: Fn(FixEvent),
{
    let mut outcome = FixOutcome::default();
    let Some(repo) = plans.first().and_then(|p| p.repo.clone()).map(PathBuf::from) else {
        outcome.error = Some("Fixing code needs a git repository, so Mehen can undo the changes if the fix does not work.".into());
        return outcome;
    };
    for edit in plans.iter().flat_map(|p| &p.edits) {
        if std::fs::read_to_string(&edit.path).ok().as_deref() != Some(edit.before.as_str()) {
            outcome.error = Some(format!("{} changed since this update was planned. Plan it again.", edit.path));
            return outcome;
        }
    }

    // What was already changed before anything ran, kept byte for byte so it
    // can be put back exactly if the tool edits it too.
    let before = changed_files(&repo);
    let kept: Vec<(PathBuf, Option<Vec<u8>>)> = before.keys().map(|p| (repo.join(p), std::fs::read(repo.join(p)).ok())).collect();
    let mut touched: Vec<String> = plans.iter().flat_map(update::touched_paths).collect();
    touched.dedup();
    let snapshots = update::snapshot(&touched);
    // Relative and with `/`, as git prints them.
    let protected: Vec<String> = touched.iter().map(|p| Path::new(p).strip_prefix(&repo).map(|r| r.display().to_string()).unwrap_or_else(|_| p.clone()).replace('\\', "/")).collect();
    let (installs, checks) = steps_of(&plans, options.build, options.test);

    let on_event = &on_event;
    let run = |step: Step| {
        let label = step.label.clone();
        on_event(FixEvent::Step { label: label.clone(), state: "running".into() });
        let started = std::time::Instant::now();
        let fut = run_step(step.clone(), cancel.clone());
        async move {
            let (ok, output) = fut.await;
            on_event(FixEvent::Step { label: label.clone(), state: if ok { "ok" } else { "failed" }.into() });
            (StepResult { label, kind: step.kind, ok, output: update::tail(&output), ms: started.elapsed().as_millis() as u64 }, output)
        }
    };

    let mut failure: Option<String> = None;
    for edit in plans.iter().flat_map(|p| &p.edits) {
        if let Err(e) = std::fs::write(&edit.path, &edit.after) {
            failure = Some(format!("Could not write {}: {e}", edit.path));
        }
    }
    if failure.is_none() {
        for step in &installs {
            let (result, _) = run(step.clone()).await;
            let ok = result.ok;
            outcome.steps.push(result);
            if !ok {
                failure = Some(format!("`{}` failed, so there was nothing to fix yet", step.label));
                break;
            }
        }
    }

    let mut earlier: Vec<String> = Vec::new();
    while failure.is_none() && !cancel.is_cancelled() {
        outcome.steps.retain(|s| s.kind == StepKind::Install);
        let mut failing: Vec<(String, String)> = Vec::new();
        for step in &checks {
            let (result, output) = run(step.clone()).await;
            if !result.ok {
                failing.push((step.label.clone(), output));
            }
            outcome.steps.push(result);
        }
        if failing.is_empty() {
            outcome.ok = true;
            break;
        }
        if outcome.rounds >= options.rounds {
            failure = Some(format!("{} could not make {} pass after {} tries", options.tool, failing.iter().map(|(l, _)| format!("`{l}`")).collect::<Vec<_>>().join(" and "), outcome.rounds));
            break;
        }
        outcome.rounds += 1;
        on_event(FixEvent::Asking { round: outcome.rounds, of: options.rounds });
        match agent(prompt(&plans, &failing, &protected, &checks, &earlier), cancel.clone()).await {
            Ok(summary) => outcome.summary = summary,
            Err(e) => failure = Some(format!("{} stopped: {e}", options.tool)),
        }
        earlier = changed_files(&repo).into_keys().filter(|p| !before.contains_key(p) && !protected.contains(p)).collect();
    }
    if cancel.is_cancelled() {
        failure = Some("Cancelled".into());
    }

    let after = changed_files(&repo);
    let manifests: BTreeSet<&String> = protected.iter().collect();
    // Files that changed now and were not changed before, besides the update's
    // own, plus earlier changes whose contents the tool rewrote.
    let mut changed: Vec<String> = after.keys().filter(|p| !before.contains_key(*p) && !manifests.contains(p)).cloned().collect();
    let rewrote: Vec<String> = kept.iter().filter(|(path, bytes)| std::fs::read(path).ok() != *bytes).filter_map(|(path, _)| path.strip_prefix(&repo).ok().map(|p| p.display().to_string().replace('\\', "/"))).filter(|p| !manifests.contains(p)).collect();
    changed.extend(rewrote.iter().cloned());
    outcome.changed = changed.clone();

    if let Some(reason) = failure {
        outcome.error = Some(reason);
        let mut problems: Vec<String> = Vec::new();
        if let Err(e) = update::restore(&snapshots) {
            problems.push(e);
        }
        for (path, bytes) in &kept {
            let result = match bytes {
                Some(b) if std::fs::read(path).ok().as_ref() != Some(b) => std::fs::write(path, b),
                _ => Ok(()),
            };
            if let Err(e) = result {
                problems.push(format!("{}: {e}", path.display()));
            }
        }
        for (path, tracked) in after.iter().filter(|(p, _)| !before.contains_key(*p) && !manifests.contains(p)) {
            if *tracked {
                if let Err(e) = update::git(&repo).args(["checkout", "HEAD", "--", path]).output().map_err(|e| e.to_string()).and_then(|o| if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) }) {
                    problems.push(format!("{path}: {e}"));
                }
            } else if let Err(e) = std::fs::remove_file(repo.join(path)) {
                problems.push(format!("{path}: {e}"));
            }
        }
        outcome.rolled_back = problems.is_empty();
        if !problems.is_empty() {
            outcome.error = Some(format!("{}. Putting files back also failed: {}", outcome.error.take().unwrap_or_default(), problems.join("; ")));
        }
        // The new versions are still installed; install again on the old files.
        for step in &installs {
            if !run(step.clone()).await.0.ok {
                outcome.error = Some(format!("{}. `{}` did not finish on the restored files; run it yourself before you build.", outcome.error.take().unwrap_or_default(), step.label));
                break;
            }
        }
        return outcome;
    }

    if options.commit {
        if !rewrote.is_empty() {
            outcome.commit_skipped = Some(format!("{} also changed files you had not committed ({})", options.tool, rewrote.join(", ")));
        } else {
            let mut paths = touched.clone();
            paths.extend(changed.iter().map(|p| repo.join(p).display().to_string()));
            let mut names: Vec<String> = plans.iter().flat_map(|p| &p.changes).map(|c| format!("{} {} to {}", c.name, c.from, c.to)).collect();
            names.dedup();
            let subject = format!("Updated {} {}", names.len(), if names.len() == 1 { "Dependency" } else { "Dependencies" });
            let body = if changed.is_empty() { names.join("\n") } else { format!("{}\n\nCode changed for the new versions by {}: {}", names.join("\n"), options.tool, changed.join(", ")) };
            match update::commit_paths(&repo, &paths, &[&subject, &body]) {
                Ok(hash) => outcome.committed = Some(hash),
                Err(e) => outcome.commit_error = Some(e),
            }
            if options.push && outcome.committed.is_some() {
                match update::push(&repo).await {
                    Ok(()) => outcome.pushed = true,
                    Err(e) => outcome.push_error = Some(e),
                }
            }
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Ecosystem;
    use crate::update::{FileEdit, PlannedChange};
    use std::cell::RefCell;

    fn git(repo: &Path, args: &[&str]) {
        let ok = update::git(repo).args(args).output().unwrap().status.success();
        assert!(ok, "git {args:?}");
    }

    /// A repository with a manifest and a source file that "builds" only when
    /// the source says `new` while the manifest says `2.0.0`.
    fn repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mehen-fix-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("package.json"), "1.0.0").unwrap();
        std::fs::write(dir.join("src/app.ts"), "old").unwrap();
        std::fs::write(dir.join("notes.txt"), "mine").unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["-c", "user.email=t@t", "-c", "user.name=t", "add", "-A"]);
        git(&dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]);
        dir
    }

    fn plan(dir: &Path) -> UpdatePlan {
        let manifest = dir.join("package.json").display().to_string();
        let step = |kind, label: &str| Step { kind, label: label.into(), program: "npm".into(), args: vec![label.into()], cwd: dir.display().to_string() };
        UpdatePlan {
            project_id: manifest.clone(),
            project_name: "app".into(),
            ecosystem: Ecosystem::Npm,
            changes: vec![PlannedChange { name: "lib".into(), from: "1.0.0".into(), to: "2.0.0".into(), written_before: "1.0.0".into(), written_after: "2.0.0".into() }],
            edits: vec![FileEdit { path: manifest, before: "1.0.0".into(), after: "2.0.0".into(), diff: String::new() }],
            steps: vec![step(StepKind::Install, "install"), step(StepKind::Verify, "build")],
            snapshots: Vec::new(),
            warnings: Vec::new(),
            repo: Some(dir.display().to_string()),
            commit_blocked: None,
            uncommitted: Vec::new(),
            branch: None,
            clean_retry: Vec::new(),
            pinned: Vec::new(),
            accepted_failures: Vec::new(),
        }
    }

    fn options(commit: bool) -> FixOptions {
        FixOptions { tool: "Claude Code".into(), build: true, test: false, commit, push: false, rounds: 2 }
    }

    fn build_step(dir: PathBuf) -> impl Fn(Step, Cancel) -> std::future::Ready<(bool, String)> {
        move |s: Step, _| {
            let source = std::fs::read_to_string(dir.join("src/app.ts")).unwrap();
            let manifest = std::fs::read_to_string(dir.join("package.json")).unwrap();
            let ok = s.kind == StepKind::Install || manifest == "1.0.0" || source == "new";
            std::future::ready((ok, if ok { String::new() } else { "src/app.ts(1,1): error TS2339: Property 'old' does not exist.".into() }))
        }
    }

    #[tokio::test]
    async fn the_tool_fixes_the_code_and_it_is_committed() {
        let dir = repo("fixed");
        let prompts = RefCell::new(Vec::new());
        let agent = |prompt: String, _| {
            prompts.borrow_mut().push(prompt);
            std::fs::write(dir.join("src/app.ts"), "new").unwrap();
            std::future::ready(Ok::<_, String>("Renamed old to new.".to_string()))
        };
        let outcome = fix(vec![plan(&dir)], options(true), &Cancel::default(), build_step(dir.clone()), agent, |_| {}).await;
        assert!(outcome.ok, "{:?}", outcome.error);
        assert_eq!((outcome.rounds, outcome.changed.clone()), (1, vec!["src/app.ts".to_string()]));
        assert!(outcome.committed.is_some(), "{:?} {:?}", outcome.commit_error, outcome.commit_skipped);
        let prompt = &prompts.borrow()[0];
        assert!(prompt.contains("- lib 1.0.0 to 2.0.0") && prompt.contains("error TS2339") && prompt.contains("package.json"), "{prompt}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_fix_that_never_works_puts_everything_back() {
        let dir = repo("unfixed");
        std::fs::write(dir.join("notes.txt"), "mine, edited").unwrap();
        let agent = |_: String, _| {
            std::fs::write(dir.join("src/app.ts"), "still wrong").unwrap();
            std::fs::write(dir.join("src/helper.ts"), "added").unwrap();
            std::fs::write(dir.join("notes.txt"), "the tool was here").unwrap();
            std::future::ready(Ok::<_, String>("Tried.".to_string()))
        };
        let outcome = fix(vec![plan(&dir)], options(true), &Cancel::default(), build_step(dir.clone()), agent, |_| {}).await;
        assert!(!outcome.ok && outcome.rolled_back, "{:?}", outcome.error);
        assert_eq!(outcome.rounds, 2);
        assert_eq!(std::fs::read_to_string(dir.join("package.json")).unwrap(), "1.0.0");
        assert_eq!(std::fs::read_to_string(dir.join("src/app.ts")).unwrap(), "old");
        assert!(!dir.join("src/helper.ts").exists(), "a file the tool added is removed");
        assert_eq!(std::fs::read_to_string(dir.join("notes.txt")).unwrap(), "mine, edited", "the user's own uncommitted edit survives");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn nothing_to_fix_when_the_update_already_works() {
        let dir = repo("works");
        let asked = std::cell::Cell::new(false);
        let agent = |_: String, _| {
            asked.set(true);
            std::future::ready(Ok::<_, String>(String::new()))
        };
        let pass = |_: Step, _| std::future::ready((true, String::new()));
        let outcome = fix(vec![plan(&dir)], options(false), &Cancel::default(), pass, agent, |_| {}).await;
        assert!(outcome.ok && !asked.get() && outcome.rounds == 0);
        assert_eq!(std::fs::read_to_string(dir.join("package.json")).unwrap(), "2.0.0");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
