//! Updating many projects at once. Plans are grouped by repository: each
//! repository's work runs in order (one working tree, one commit), while
//! different repositories run side by side. Steps that use the same tool take
//! turns (two `npm install`s at once tread on each other), and a global limit
//! keeps the machine usable.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Instant;

use futures::future::join_all;
use serde::Serialize;
use tokio::sync::{Mutex, Semaphore};

use crate::update::{self, Step, StepKind, StepResult, UpdatePlan};

/// How many steps may run at once across all repositories.
// TODO: pick this from the machine (cores, free memory) instead of a fixed default.
pub const DEFAULT_PARALLEL: usize = 2;

#[derive(Debug, Clone, Copy)]
pub struct BatchOptions {
    /// Run build and test steps, not just installs.
    pub checks: bool,
    /// Commit each repository once its work succeeds.
    pub commit: bool,
    /// Steps running at once across every repository; at least 1.
    pub parallel: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobState {
    Queued,
    /// Waiting for another repository to finish with the same tool.
    Waiting,
    Running,
    Committing,
    Done,
    Failed,
    RolledBack,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchEvent {
    /// The repository (or project folder) this job updates.
    pub job: String,
    pub projects: Vec<String>,
    pub state: JobState,
    /// The step running, or what the job waits for.
    pub label: Option<String>,
    /// The tool the step uses (`npm`, `dotnet`, ...).
    pub lane: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobOutcome {
    pub job: String,
    pub name: String,
    pub repo: Option<String>,
    pub projects: Vec<String>,
    pub ok: bool,
    pub rolled_back: bool,
    pub error: Option<String>,
    pub steps: Vec<StepResult>,
    /// Short hash of the commit made for this repository.
    pub committed: Option<String>,
    /// The update stayed applied but the commit failed (a hook, for example).
    pub commit_error: Option<String>,
    /// Why no commit was attempted although one was asked for.
    pub commit_skipped: Option<String>,
}

/// One repository's share of the batch.
pub struct Job {
    pub key: String,
    pub name: String,
    pub repo: Option<PathBuf>,
    pub plans: Vec<UpdatePlan>,
}

impl Job {
    fn projects(&self) -> Vec<String> {
        self.plans.iter().map(|p| p.project_id.clone()).collect()
    }

    fn event(&self, state: JobState, label: Option<String>, lane: Option<String>) -> BatchEvent {
        BatchEvent { job: self.key.clone(), projects: self.projects(), state, label, lane }
    }

    fn touched_paths(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        self.plans.iter().flat_map(update::touched_paths).filter(|p| seen.insert(p.to_lowercase())).collect()
    }

    /// Installs first (every manifest is edited before anything builds), then
    /// builds, then tests. A step two projects share (one lockfile at the
    /// repository root) runs once.
    fn steps(&self, checks: bool) -> Vec<Step> {
        let mut seen = HashSet::new();
        let mut steps = Vec::new();
        for kind in [StepKind::Install, StepKind::Verify, StepKind::Test] {
            if kind.is_check() && !checks {
                continue;
            }
            for step in self.plans.iter().flat_map(|p| &p.steps).filter(|s| s.kind == kind) {
                if seen.insert((step.program.to_lowercase(), step.args.clone(), step.cwd.to_lowercase())) {
                    steps.push(step.clone());
                }
            }
        }
        steps
    }

    /// `Updated 2 Dependencies`, then one line per package.
    fn commit_message(&self) -> (String, String) {
        let mut changes: BTreeMap<String, (String, String)> = BTreeMap::new();
        for c in self.plans.iter().flat_map(|p| &p.changes) {
            changes.entry(c.name.clone()).or_insert_with(|| (c.from.clone(), c.to.clone()));
        }
        let n = changes.len();
        let subject = format!("Updated {n} {}", if n == 1 { "Dependency" } else { "Dependencies" });
        let body = changes.iter().map(|(name, (from, to))| format!("{name} {from} to {to}")).collect::<Vec<_>>().join("\n");
        (subject, body)
    }

    fn commit_blocked(&self) -> Option<String> {
        if self.repo.is_none() {
            return Some("not inside a git repository".into());
        }
        let reasons: Vec<&str> = self.plans.iter().filter_map(|p| p.commit_blocked.as_deref()).collect();
        (!reasons.is_empty()).then(|| reasons.join("; "))
    }
}

/// The step's tool, which decides who it must take turns with.
pub fn lane(step: &Step) -> String {
    let program = step.program.to_lowercase();
    program.trim_end_matches(".cmd").trim_end_matches(".exe").to_string()
}

fn folder_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string())
}

/// Groups plans by repository, or by project folder outside a repository.
/// Keeps the order plans arrived in.
pub fn group(plans: Vec<UpdatePlan>) -> Vec<Job> {
    let mut jobs: Vec<Job> = Vec::new();
    for plan in plans {
        let key = plan.repo.clone().unwrap_or_else(|| {
            Path::new(&plan.project_id).parent().map(|p| p.display().to_string()).unwrap_or_else(|| plan.project_id.clone())
        });
        match jobs.iter_mut().find(|j| j.key.eq_ignore_ascii_case(&key)) {
            Some(job) => job.plans.push(plan),
            None => jobs.push(Job { name: folder_name(&key), repo: plan.repo.as_ref().map(PathBuf::from), key, plans: vec![plan] }),
        }
    }
    jobs
}

/// Runs every plan. `run_step` executes one command (tests pass a fake);
/// `on_event` reports progress per repository.
pub async fn run<R, F, E>(plans: Vec<UpdatePlan>, options: BatchOptions, run_step: R, on_event: E) -> Vec<JobOutcome>
where
    R: Fn(Step) -> F,
    F: Future<Output = (bool, String)>,
    E: Fn(BatchEvent),
{
    let jobs = group(plans);
    let steps: Vec<Vec<Step>> = jobs.iter().map(|j| j.steps(options.checks)).collect();
    let lanes: HashMap<String, Mutex<()>> = steps.iter().flatten().map(|s| (lane(s), Mutex::new(()))).collect();
    let limit = Semaphore::new(options.parallel.max(1));
    for job in &jobs {
        on_event(job.event(JobState::Queued, None, None));
    }
    join_all(jobs.iter().zip(steps).map(|(job, steps)| run_job(job, steps, options, &lanes, &limit, &run_step, &on_event))).await
}

async fn run_job<R, F, E>(
    job: &Job,
    steps: Vec<Step>,
    options: BatchOptions,
    lanes: &HashMap<String, Mutex<()>>,
    limit: &Semaphore,
    run_step: &R,
    on_event: &E,
) -> JobOutcome
where
    R: Fn(Step) -> F,
    F: Future<Output = (bool, String)>,
    E: Fn(BatchEvent),
{
    let mut outcome = JobOutcome {
        job: job.key.clone(),
        name: job.name.clone(),
        repo: job.repo.as_ref().map(|r| r.display().to_string()),
        projects: job.projects(),
        ok: false,
        rolled_back: false,
        error: None,
        steps: Vec::new(),
        committed: None,
        commit_error: None,
        commit_skipped: None,
    };
    let fail = |outcome: &mut JobOutcome, error: String| {
        outcome.error = Some(error.clone());
        on_event(job.event(JobState::Failed, Some(error), None));
    };

    for edit in job.plans.iter().flat_map(|p| &p.edits) {
        match std::fs::read_to_string(&edit.path) {
            Ok(current) if current == edit.before => {}
            Ok(_) => {
                fail(&mut outcome, format!("{} changed since this update was reviewed. Review it again.", edit.path));
                return outcome;
            }
            Err(e) => {
                fail(&mut outcome, format!("{}: {e}", edit.path));
                return outcome;
            }
        }
    }

    let paths = job.touched_paths();
    let snapshots = update::snapshot(&paths);
    let roll_back = |outcome: &mut JobOutcome, reason: String| {
        match update::restore(&snapshots) {
            Ok(()) => outcome.rolled_back = true,
            Err(e) => outcome.error = Some(format!("{reason}. Restoring files also failed: {e}")),
        }
        if outcome.error.is_none() {
            outcome.error = Some(reason.clone());
        }
        on_event(job.event(if outcome.rolled_back { JobState::RolledBack } else { JobState::Failed }, Some(reason), None));
    };

    for edit in job.plans.iter().flat_map(|p| &p.edits) {
        if let Err(e) = std::fs::write(&edit.path, &edit.after) {
            roll_back(&mut outcome, format!("Could not write {}: {e}", edit.path));
            return outcome;
        }
    }

    for step in steps {
        let lane = lane(&step);
        let lane_lock = &lanes[&lane];
        let _turn = match lane_lock.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                on_event(job.event(JobState::Waiting, Some(format!("Waiting for {lane}")), Some(lane.clone())));
                lane_lock.lock().await
            }
        };
        let _permit = match limit.try_acquire() {
            Ok(permit) => permit,
            Err(_) => {
                on_event(job.event(JobState::Waiting, Some("Waiting for a free slot".into()), Some(lane.clone())));
                limit.acquire().await.expect("the batch semaphore is never closed")
            }
        };
        on_event(job.event(JobState::Running, Some(step.label.clone()), Some(lane.clone())));
        let started = Instant::now();
        let (ok, output) = run_step(step.clone()).await;
        outcome.steps.push(StepResult { label: step.label.clone(), kind: step.kind, ok, output, ms: started.elapsed().as_millis() as u64 });
        if !ok {
            roll_back(&mut outcome, format!("`{}` failed", step.label));
            return outcome;
        }
    }

    outcome.ok = true;
    if options.commit {
        match (job.commit_blocked(), &job.repo) {
            (Some(reason), _) => outcome.commit_skipped = Some(reason),
            (None, None) => outcome.commit_skipped = Some("not inside a git repository".into()),
            (None, Some(repo)) => {
                on_event(job.event(JobState::Committing, Some("git commit".into()), None));
                let (subject, body) = job.commit_message();
                match update::commit_paths(repo, &paths, &[&subject, &body]) {
                    Ok(hash) => outcome.committed = Some(hash),
                    Err(e) => outcome.commit_error = Some(e),
                }
            }
        }
    }
    on_event(job.event(JobState::Done, outcome.committed.clone(), None));
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Ecosystem;
    use crate::update::{FileEdit, PlannedChange};
    use std::cell::RefCell;
    use std::time::Duration;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mehen-batch-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn step(program: &str, kind: StepKind, cwd: &Path) -> Step {
        Step { kind, label: format!("{program} {kind:?}"), program: program.into(), args: vec![format!("{kind:?}")], cwd: cwd.display().to_string() }
    }

    /// A plan for `dir/manifest` whose edit replaces `before` with `after`.
    fn plan(dir: &Path, manifest: &str, repo: Option<&Path>, steps: Vec<Step>) -> UpdatePlan {
        let path = dir.join(manifest);
        std::fs::write(&path, "before").unwrap();
        UpdatePlan {
            project_id: path.display().to_string(),
            project_name: manifest.into(),
            ecosystem: Ecosystem::Npm,
            changes: vec![PlannedChange { name: format!("{manifest}-dep"), from: "1.0.0".into(), to: "2.0.0".into(), written_before: "1.0.0".into(), written_after: "2.0.0".into() }],
            edits: vec![FileEdit { path: path.display().to_string(), before: "before".into(), after: "after".into(), diff: String::new() }],
            steps,
            snapshots: Vec::new(),
            warnings: Vec::new(),
            repo: repo.map(|r| r.display().to_string()),
            commit_blocked: None,
        }
    }

    type Log = RefCell<Vec<(String, Instant, Instant)>>;

    /// Records when each step ran, by lane; `fails` names a program that fails.
    fn recorder<'a>(log: &'a Log, fails: &'a str) -> impl Fn(Step) -> std::pin::Pin<Box<dyn Future<Output = (bool, String)> + 'a>> + 'a {
        move |s: Step| {
            Box::pin(async move {
                let start = Instant::now();
                tokio::time::sleep(Duration::from_millis(40)).await;
                log.borrow_mut().push((lane(&s), start, Instant::now()));
                (s.program != fails, String::new())
            })
        }
    }

    fn overlaps(a: &(String, Instant, Instant), b: &(String, Instant, Instant)) -> bool {
        a.1 < b.2 && b.1 < a.2
    }

    #[tokio::test]
    async fn same_tool_takes_turns_other_tools_run_together() {
        let dir = temp("lanes");
        let (a, b, c) = (dir.join("a"), dir.join("b"), dir.join("c"));
        for d in [&a, &b, &c] {
            std::fs::create_dir_all(d).unwrap();
        }
        let plans = || {
            vec![
                plan(&a, "package.json", Some(&a), vec![step("npm", StepKind::Install, &a)]),
                plan(&b, "package.json", Some(&b), vec![step("npm", StepKind::Install, &b)]),
                plan(&c, "app.csproj", Some(&c), vec![step("dotnet", StepKind::Install, &c)]),
            ]
        };

        let log = Log::default();
        let opts = BatchOptions { checks: true, commit: false, parallel: 2 };
        let outcomes = run(plans(), opts, recorder(&log, ""), |_| {}).await;
        assert!(outcomes.iter().all(|o| o.ok));
        let runs = log.borrow();
        let npm: Vec<_> = runs.iter().filter(|r| r.0 == "npm").collect();
        let dotnet = runs.iter().find(|r| r.0 == "dotnet").unwrap();
        assert!(!overlaps(npm[0], npm[1]), "two npm installs overlapped");
        assert!(npm.iter().any(|n| overlaps(n, dotnet)), "npm and dotnet never ran together");

        let log = Log::default();
        let opts = BatchOptions { parallel: 1, ..opts };
        run(plans(), opts, recorder(&log, ""), |_| {}).await;
        let runs = log.borrow();
        for (i, x) in runs.iter().enumerate() {
            for y in &runs[i + 1..] {
                assert!(!overlaps(x, y), "a limit of 1 still ran steps together");
            }
        }
    }

    #[tokio::test]
    async fn one_job_per_repo_with_shared_steps_run_once() {
        let dir = temp("group");
        let tauri = dir.join("src-tauri");
        std::fs::create_dir_all(&tauri).unwrap();
        let install = step("npm", StepKind::Install, &dir);
        let plans = vec![
            plan(&dir, "package.json", Some(&dir), vec![install.clone(), step("npm", StepKind::Verify, &dir)]),
            plan(&tauri, "Cargo.toml", Some(&dir), vec![step("cargo", StepKind::Install, &tauri), step("cargo", StepKind::Test, &tauri)]),
            plan(&dir, "other.json", Some(&dir), vec![install]),
        ];
        let jobs = group(plans.clone());
        assert_eq!(jobs.len(), 1);
        let order: Vec<(String, StepKind)> = jobs[0].steps(true).iter().map(|s| (s.program.clone(), s.kind)).collect();
        assert_eq!(order, vec![("npm".into(), StepKind::Install), ("cargo".into(), StepKind::Install), ("npm".into(), StepKind::Verify), ("cargo".into(), StepKind::Test)]);
        assert_eq!(jobs[0].steps(false).len(), 2, "checks off keeps only installs");

        let log = Log::default();
        let outcomes = run(plans, BatchOptions { checks: false, commit: false, parallel: 2 }, recorder(&log, ""), |_| {}).await;
        assert_eq!(outcomes.len(), 1);
        assert_eq!(log.borrow().len(), 2);
        assert_eq!(std::fs::read_to_string(dir.join("package.json")).unwrap(), "after");
    }

    #[tokio::test]
    async fn a_failure_restores_the_whole_repo_only() {
        let dir = temp("rollback");
        let (good, bad) = (dir.join("good"), dir.join("bad"));
        std::fs::create_dir_all(bad.join("web")).unwrap();
        std::fs::create_dir_all(&good).unwrap();
        let plans = vec![
            plan(&bad, "Cargo.toml", Some(&bad), vec![step("cargo", StepKind::Install, &bad)]),
            plan(&bad.join("web"), "package.json", Some(&bad), vec![step("npm", StepKind::Test, &bad)]),
            plan(&good, "package.json", Some(&good), vec![step("pnpm", StepKind::Install, &good)]),
        ];
        let events = RefCell::new(Vec::new());
        let log = Log::default();
        let outcomes = run(plans, BatchOptions { checks: true, commit: false, parallel: 2 }, recorder(&log, "npm"), |e| events.borrow_mut().push(e)).await;
        let bad_outcome = outcomes.iter().find(|o| o.name == "bad").unwrap();
        assert!(!bad_outcome.ok && bad_outcome.rolled_back, "{:?}", bad_outcome.error);
        assert_eq!(std::fs::read_to_string(bad.join("Cargo.toml")).unwrap(), "before");
        assert_eq!(std::fs::read_to_string(bad.join("web").join("package.json")).unwrap(), "before");
        assert!(outcomes.iter().find(|o| o.name == "good").unwrap().ok);
        assert_eq!(std::fs::read_to_string(good.join("package.json")).unwrap(), "after");
        assert!(events.borrow().iter().any(|e| e.state == JobState::RolledBack && e.projects.len() == 2));
    }

    #[tokio::test]
    async fn commits_each_repo_with_a_count() {
        let dir = temp("commit");
        let git = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "test"]);
        std::fs::write(dir.join("package.json"), "before").unwrap();
        std::fs::write(dir.join("app.csproj"), "before").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);

        let plans = vec![plan(&dir, "package.json", Some(&dir), vec![]), plan(&dir, "app.csproj", Some(&dir), vec![])];
        let outcomes = run(plans, BatchOptions { checks: true, commit: true, parallel: 2 }, |_| async { (true, String::new()) }, |_| {}).await;
        assert!(outcomes[0].committed.is_some(), "{:?}", outcomes[0].commit_error);
        let log = String::from_utf8(git(&["log", "-1", "--format=%s%n%b"]).stdout).unwrap();
        assert!(log.starts_with("Updated 2 Dependencies"), "{log}");
        assert!(log.contains("app.csproj-dep 1.0.0 to 2.0.0") && log.contains("package.json-dep 1.0.0 to 2.0.0"), "{log}");
        let files = String::from_utf8(git(&["show", "--name-only", "--format=", "HEAD"]).stdout).unwrap();
        assert_eq!(files.lines().count(), 2, "{files}");
    }
}
