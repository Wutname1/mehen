use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mehen_core::cancel::Cancels;
use mehen_core::check;
use mehen_core::batch::{self, BatchEvent, BatchOptions, CommitOutcome, JobOutcome};
use mehen_core::status::Fresh;
use mehen_core::store::{Hold, StoreStats};
use mehen_core::update::{self, Change, UpdatePlan};
use mehen_core::{DiscoveredProject, Ecosystem, IgnoreKind, IgnoreRule, IgnoreSet, Inventory, Progress, Project, Store};
use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_notification::NotificationExt;

mod background;
mod gitwyrm;
#[cfg(target_os = "macos")]
mod macos_env;
mod scrub;
mod self_update;
mod telemetry;

pub use background::requested as background_check_requested;
pub use background::run as run_background_check;

/// Setting key: hours between background checks; 0 or missing means off.
const BACKGROUND_HOURS: &str = "background_hours";
/// Setting key: how many update steps may run at once across repositories.
const UPDATE_PARALLEL: &str = "update_parallel";
/// Setting key: JSON map of scope to the commands that replace a project's
/// build and test steps. A scope is a repository path, or `ecosystem:<name>`
/// for the default of every project of that kind.
const CHECK_COMMANDS: &str = "check_commands";
/// Setting key: JSON map of scope (a repository path, or `*` for every
/// project) to the newest kind of release updates may move to.
const VERSION_POLICY: &str = "version_policy";
/// Setting key: `false` turns off notifications about new vulnerabilities.
const NOTIFY: &str = "notify_vulnerabilities";
/// Setting key: `false` stops Mehen checking for new versions of itself.
const APP_UPDATE_CHECK: &str = "app_update_check";
/// How often the background loop wakes to see whether a check is due.
const BACKGROUND_TICK: Duration = Duration::from_secs(10 * 60);

struct AppState {
    store: Store,
    /// Set while a check runs, so background and manual checks never overlap.
    checking: AtomicBool,
    /// Stop switches for the update running now.
    cancels: std::sync::Mutex<Arc<Cancels>>,
    /// The last result this app ran or loaded, for when the saved one cannot
    /// be read back.
    inventory: std::sync::Mutex<Option<Inventory>>,
}

impl AppState {
    fn remember(&self, inventory: &Inventory) {
        *self.inventory.lock().unwrap_or_else(|e| e.into_inner()) = Some(inventory.clone());
    }

    fn remembered(&self) -> Option<Inventory> {
        self.inventory.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The newest result: the saved one, or the one in memory when the saved
    /// one cannot be read.
    fn last_result(&self) -> Option<Inventory> {
        self.store.last_inventory().or_else(|| self.remembered())
    }

    /// A project as the last check saw it. Falls back to the result in memory
    /// when the saved one cannot be read or lacks the project, and otherwise
    /// says why there is nothing to go on.
    fn project(&self, id: &str) -> Result<Project, String> {
        let find = |inventory: &Inventory| inventory.projects.iter().find(|p| p.id == id).cloned();
        let saved = self.store.read_last_inventory();
        if let Some(project) = saved.as_ref().ok().and_then(|i| i.as_ref()).and_then(find).or_else(|| self.remembered().as_ref().and_then(find)) {
            return Ok(project);
        }
        Err(match saved {
            Err(e) => format!("Mehen could not read its last check ({e:#}). Check again, then try once more."),
            Ok(None) => "Run a check first".into(),
            Ok(Some(_)) => "That project is not in the last check. Check again first.".into(),
        })
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    folders: Vec<String>,
    rules: Vec<IgnoreRule>,
    background_hours: u32,
    /// Steps running at once; 0 means automatic.
    update_parallel: usize,
    /// What automatic resolves to on this computer.
    update_parallel_auto: usize,
    notify: bool,
    app_update_check: bool,
    /// Windows runs a quick check at sign-in and daily, with Mehen closed.
    scheduled_check: bool,
    scheduled_check_supported: bool,
    error_reports: bool,
}

/// The user's own build and test commands for one scope, and where to run them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckConfig {
    commands: Vec<String>,
    /// A folder relative to the repository (or project), or an absolute one.
    #[serde(default)]
    cwd: Option<String>,
}

impl AppState {
    fn settings(&self) -> Settings {
        Settings {
            folders: self.store.folders(),
            rules: self.store.ignore_rules(),
            background_hours: self.background_hours(),
            update_parallel: self.store.setting(UPDATE_PARALLEL).and_then(|v| v.parse().ok()).unwrap_or(0),
            update_parallel_auto: batch::auto_parallel(),
            notify: self.notify(),
            app_update_check: self.store.setting(APP_UPDATE_CHECK).is_none_or(|v| v != "false"),
            scheduled_check: self.scheduled_check(),
            scheduled_check_supported: background::schedule_supported(),
            error_reports: self.error_reports(),
        }
    }

    /// On unless turned off in settings.
    fn error_reports(&self) -> bool {
        self.store.setting(telemetry::ERROR_REPORTS).is_none_or(|v| v != "false")
    }

    /// Off unless turned on in settings.
    fn scheduled_check(&self) -> bool {
        self.store.setting(background::SCHEDULED_CHECK).as_deref() == Some("true")
    }

    fn roots(&self) -> Vec<PathBuf> {
        self.store.folders().into_iter().map(PathBuf::from).collect()
    }

    /// Steps allowed at once, with 0 (automatic) resolved for this computer.
    fn update_parallel(&self) -> usize {
        match self.store.setting(UPDATE_PARALLEL).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0) {
            0 => batch::auto_parallel(),
            n => n.clamp(1, 8),
        }
    }

    fn notify(&self) -> bool {
        self.store.setting(NOTIFY).is_none_or(|v| v != "false")
    }

    /// Accepts the older plain list of commands as well as the full config.
    fn check_commands(&self) -> BTreeMap<String, CheckConfig> {
        let raw: BTreeMap<String, serde_json::Value> = self.store.setting(CHECK_COMMANDS).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
        raw.into_iter()
            .filter_map(|(scope, value)| {
                let config = match value {
                    serde_json::Value::Array(_) => CheckConfig { commands: serde_json::from_value(value).ok()?, cwd: None },
                    other => serde_json::from_value(other).ok()?,
                };
                Some((scope, config))
            })
            .collect()
    }

    fn version_policy(&self) -> BTreeMap<String, String> {
        self.store.setting(VERSION_POLICY).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    fn background_hours(&self) -> u32 {
        self.store.setting(BACKGROUND_HOURS).and_then(|v| v.parse().ok()).unwrap_or(0)
    }
}

/// Sends a damaged-database report with the end of the database trace, so
/// the report shows which processes were working on the file just before.
fn report_damage(message: &str) {
    let trace = mehen_core::trace::tail(200);
    sentry::with_scope(|scope| scope.set_extra("db_trace", trace.into()), || sentry::capture_message(message, sentry::Level::Error));
}

/// Scans the watched folders (minus ignored ones) and checks every package.
/// With `only`, scans just those folders and folds the result into the last one.
async fn check_now(app: &AppHandle, refresh: bool, only: Option<Vec<String>>) -> Result<Inventory, String> {
    let state = app.state::<AppState>();
    let dir = app.path().app_data_dir().map_err(err)?;
    // A background run (started by GitWyrm or the daily task) may hold it.
    let _lock = background::CheckLock::take(&dir, Duration::from_secs(30)).await.ok_or("A check is already running")?;
    let emit = |p: Progress| {
        let _ = app.emit("mehen://progress", p);
    };
    let full = only.is_none();
    mehen_core::trace::line(format!("check start: {}", only.as_ref().map_or_else(|| "full".to_string(), |o| format!("only {o:?}"))));
    let started = std::time::Instant::now();
    let checked = background::run_check(&state.store, &dir, refresh, only, state.remembered(), emit).await;
    mehen_core::trace::line(format!("check end: {} in {} ms", if checked.is_ok() { "ok" } else { "failed" }, started.elapsed().as_millis()));
    let checked = checked?;
    state.remember(&checked);
    let health = state.store.health();
    if !health.ok {
        report_damage(&format!("Mehen's saved data was damaged during a check: {}", health.detail));
    }
    if full {
        // Only a full check knows everything still in use. Awaited, so the
        // cleanup finishes while this check still holds the lock and no
        // background run is writing at the same time.
        let app = app.clone();
        let snapshot = checked.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            match app.state::<AppState>().store.prune(&snapshot) {
                Ok(pruned) => mehen_core::trace::line(format!("pruned: {pruned:?}")),
                Err(e) => {
                    mehen_core::trace::line(format!("prune failed: {e:#}"));
                    eprintln!("Cleaning the cache failed: {e:#}");
                }
            }
        })
        .await;
    }
    Ok(checked)
}

/// Rewrites the per-repository summary other apps read, such as GitWyrm's
/// dependency status. Failing to write it never fails the check.
fn publish_status(app: &AppHandle, inventory: &Inventory, fresh: Fresh) {
    if let Ok(dir) = app.path().app_data_dir() {
        background::publish(&dir, inventory, fresh);
    }
}

/// Runs a check unless one is already going.
async fn guarded_check(app: &AppHandle, refresh: bool, only: Option<Vec<String>>) -> Result<Inventory, String> {
    let state = app.state::<AppState>();
    if state.checking.swap(true, Ordering::SeqCst) {
        return Err("A check is already running".into());
    }
    let result = check_now(app, refresh, only).await;
    state.checking.store(false, Ordering::SeqCst);
    result
}

/// A check started from the tray or the background loop: shows the result in
/// the window and raises a notification for vulnerabilities that are new
/// since the previous result.
async fn check_and_notify(app: &AppHandle) {
    if !app.state::<AppState>().notify() {
        let _ = guarded_check(app, false, None).await.map(|inventory| app.emit("mehen://inventory", &inventory));
        return;
    }
    let before: HashSet<String> = app.state::<AppState>().store.last_inventory().map(|i| i.vulnerabilities.into_iter().map(|v| v.id).collect()).unwrap_or_default();
    let Ok(inventory) = guarded_check(app, false, None).await else { return };
    let _ = app.emit("mehen://inventory", &inventory);

    let new: Vec<_> = inventory.vulnerabilities.iter().filter(|v| !before.contains(&v.id)).collect();
    if new.is_empty() {
        return;
    }
    let affected = |id: &str| -> Vec<String> {
        let mut names: Vec<String> =
            inventory.projects.iter().filter(|p| p.dependencies.iter().any(|d| d.vulns.iter().any(|v| v == id))).map(|p| p.name.clone()).collect();
        names.dedup();
        names
    };
    let title = if new.len() == 1 { "1 new vulnerability".to_string() } else { format!("{} new vulnerabilities", new.len()) };
    let body = new
        .iter()
        .take(3)
        .map(|v| {
            let projects = affected(&v.id);
            let severity = v.severity.as_deref().map(|s| format!("{s}: ")).unwrap_or_default();
            format!("{severity}{} ({})", v.summary.chars().take(70).collect::<String>(), projects.first().cloned().unwrap_or_default())
        })
        .collect::<Vec<_>>()
        .join("\n");
    let _ = app.notification().builder().title(format!("Mehen: {title}")).body(body).show();
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Wakes every few minutes and runs a check when the last one is older than
/// the chosen interval. Uses the cache, so most runs make few network calls.
fn start_background_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            let (hours, last) = {
                let state = app.state::<AppState>();
                (state.background_hours(), state.store.last_inventory().and_then(|i| i.checked_at).unwrap_or(0))
            };
            let has_folders = !app.state::<AppState>().store.folders().is_empty();
            if hours > 0 && has_folders && now_secs().saturating_sub(last) >= u64::from(hours) * 3600 {
                check_and_notify(&app).await;
            }
            tokio::time::sleep(BACKGROUND_TICK).await;
        }
    });
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Mehen", true, None::<&str>)?;
    let check = MenuItem::with_id(app, "check", "Check now", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &check, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main").tooltip("Mehen").menu(&menu).show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(|app, event| match event.id.as_ref() {
        "open" => show_window(app),
        "check" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { check_and_notify(&app).await });
        }
        "quit" => app.exit(0),
        _ => {}
    })
    .on_tray_icon_event(|tray, event| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
            show_window(tray.app_handle());
        }
    })
    .build(app)?;
    Ok(())
}

#[tauri::command]
fn settings(state: State<'_, AppState>) -> Settings {
    state.settings()
}

#[tauri::command]
fn add_folder(state: State<'_, AppState>, path: String) -> Result<Settings, String> {
    state.store.add_folder(&path).map_err(err)?;
    Ok(state.settings())
}

#[tauri::command]
fn remove_folder(state: State<'_, AppState>, path: String) -> Result<Settings, String> {
    state.store.remove_folder(&path).map_err(err)?;
    Ok(state.settings())
}

/// Hours between background checks; 0 turns them off.
#[tauri::command]
fn set_background_hours(state: State<'_, AppState>, hours: u32) -> Result<Settings, String> {
    state.store.set_setting(BACKGROUND_HOURS, &hours.to_string()).map_err(err)?;
    Ok(state.settings())
}

/// How many update steps may run at once; 1 runs one at a time, 0 is automatic.
#[tauri::command]
fn set_update_parallel(state: State<'_, AppState>, parallel: usize) -> Result<Settings, String> {
    state.store.set_setting(UPDATE_PARALLEL, &parallel.min(8).to_string()).map_err(err)?;
    Ok(state.settings())
}

/// Turns notifications about new vulnerabilities on or off.
#[tauri::command]
fn set_notify(state: State<'_, AppState>, notify: bool) -> Result<Settings, String> {
    state.store.set_setting(NOTIFY, if notify { "true" } else { "false" }).map_err(err)?;
    Ok(state.settings())
}

/// Creates or removes the Windows scheduled task, then remembers the choice.
#[tauri::command]
async fn set_scheduled_check(state: State<'_, AppState>, enabled: bool) -> Result<Settings, String> {
    tauri::async_runtime::spawn_blocking(move || background::set_scheduled(enabled)).await.map_err(err)??;
    state.store.set_setting(background::SCHEDULED_CHECK, if enabled { "true" } else { "false" }).map_err(err)?;
    Ok(state.settings())
}

#[tauri::command]
fn set_error_reports(state: State<'_, AppState>, enabled: bool) -> Result<Settings, String> {
    state.store.set_setting(telemetry::ERROR_REPORTS, if enabled { "true" } else { "false" }).map_err(err)?;
    telemetry::set_enabled(enabled);
    Ok(state.settings())
}

#[tauri::command]
fn set_app_update_check(state: State<'_, AppState>, check: bool) -> Result<Settings, String> {
    state.store.set_setting(APP_UPDATE_CHECK, if check { "true" } else { "false" }).map_err(err)?;
    Ok(state.settings())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IgnoreResult {
    settings: Settings,
    /// The last saved result with the newly ignored projects removed.
    inventory: Option<Inventory>,
}

/// Adds a rule and applies it to the saved result straight away, so ignoring
/// something after a check does not need a new check.
#[tauri::command]
fn add_ignore(app: AppHandle, state: State<'_, AppState>, kind: IgnoreKind, value: String, note: Option<String>) -> Result<IgnoreResult, String> {
    state.store.add_ignore_rule(kind, &value, note.as_deref()).map_err(err)?;
    let inventory = state.last_result().map(|mut inv| {
        IgnoreSet::new(&state.store.ignore_rules(), &state.roots()).apply(&mut inv);
        inv
    });
    if let Some(inv) = &inventory {
        state.remember(inv);
        state.store.replace_last_inventory(inv).map_err(err)?;
        publish_status(&app, inv, Fresh::None);
    }
    Ok(IgnoreResult { settings: state.settings(), inventory })
}

#[tauri::command]
fn remove_ignore(state: State<'_, AppState>, id: i64) -> Result<Settings, String> {
    state.store.remove_ignore_rule(id).map_err(err)?;
    Ok(state.settings())
}

/// Lists every project in the watched folders without touching the network,
/// marking the ones current rules would skip.
#[tauri::command]
async fn discover(state: State<'_, AppState>) -> Result<Vec<DiscoveredProject>, String> {
    let (roots, rules) = (state.roots(), state.store.ignore_rules());
    tauri::async_runtime::spawn_blocking(move || mehen_core::discover(&roots, &rules)).await.map_err(err)
}

/// The last saved result, so the app opens with data instead of a spinner.
#[tauri::command]
fn last_inventory(state: State<'_, AppState>) -> Option<Inventory> {
    state.last_result()
}

/// With `refresh`, cached registry and vulnerability answers are ignored.
/// With `only`, just those folders (repositories or watched folders) are checked.
#[tauri::command]
async fn scan_and_check(app: AppHandle, refresh: bool, only: Option<Vec<String>>) -> Result<Inventory, String> {
    guarded_check(&app, refresh, only).await
}

/// Works out exactly what an update would change, without writing anything.
#[tauri::command]
fn plan_update(state: State<'_, AppState>, project_id: String, changes: Vec<Change>) -> Result<UpdatePlan, String> {
    let project = state.project(&project_id)?;
    let mut plan = update::plan(&project, &changes, |name| state.store.package_any_age(Ecosystem::GithubActions, name)).map_err(|e| format!("{e:#}"))?;
    let commands = state.check_commands();
    let repo = project.repo.clone().unwrap_or_else(|| project.dir.clone());
    let chosen = match commands.iter().find(|(scope, _)| scope.eq_ignore_ascii_case(&repo)) {
        Some((_, config)) => Some((config, repo.as_str())),
        None => commands.get(&format!("ecosystem:{}", project.ecosystem.key())).map(|config| (config, project.dir.as_str())),
    };
    if let Some((config, base)) = chosen {
        let cwd = config.cwd.as_deref().filter(|c| !c.trim().is_empty()).map(|c| PathBuf::from(base).join(c.trim()).display().to_string()).unwrap_or_else(|| base.to_string());
        update::use_check_commands(&mut plan, &config.commands, &cwd);
    }
    Ok(plan)
}

/// Build and test commands the user set, by scope.
#[tauri::command]
fn check_commands(state: State<'_, AppState>) -> BTreeMap<String, CheckConfig> {
    state.check_commands()
}

/// Sets (or with `None`, removes) the commands for a scope.
#[tauri::command]
fn set_check_commands(state: State<'_, AppState>, scope: String, commands: Option<Vec<String>>, cwd: Option<String>) -> Result<BTreeMap<String, CheckConfig>, String> {
    let mut all = state.check_commands();
    all.retain(|k, _| !k.eq_ignore_ascii_case(&scope));
    if let Some(list) = commands {
        let commands = list.into_iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
        all.insert(scope, CheckConfig { commands, cwd: cwd.map(|c| c.trim().to_string()).filter(|c| !c.is_empty()) });
    }
    state.store.set_setting(CHECK_COMMANDS, &serde_json::to_string(&all).map_err(err)?).map_err(err)?;
    Ok(all)
}

/// How far updates may go, by scope (`*` for every project): `any`, `minor`, or `patch`.
#[tauri::command]
fn version_policy(state: State<'_, AppState>) -> BTreeMap<String, String> {
    state.version_policy()
}

/// Sets (or with `None`, removes) the policy for a scope.
#[tauri::command]
fn set_version_policy(state: State<'_, AppState>, scope: String, policy: Option<String>) -> Result<BTreeMap<String, String>, String> {
    let mut all = state.version_policy();
    all.retain(|k, _| !k.eq_ignore_ascii_case(&scope));
    if let Some(p) = policy.filter(|p| ["any", "minor", "patch"].contains(&p.as_str())) {
        all.insert(scope, p);
    }
    state.store.set_setting(VERSION_POLICY, &serde_json::to_string(&all).map_err(err)?).map_err(err)?;
    Ok(all)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RepoIcon {
    repo: String,
    data_url: String,
}

/// Logos for project folders. Remembered answers come straight from the store;
/// with `discover`, folders never searched (or whose logo file moved) are
/// searched once and the answer remembered, including "no logo".
#[tauri::command]
async fn repo_icons(app: AppHandle, repos: Vec<String>, discover: bool) -> Result<Vec<RepoIcon>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let store = &app.state::<AppState>().store;
        repos
            .into_iter()
            .filter_map(|repo| {
                let path = match store.repo_icon(&repo) {
                    Some(Some(p)) if std::path::Path::new(&p).is_file() => Some(p),
                    Some(None) => None,
                    _ if discover => {
                        let found = mehen_core::icons::find_icon(std::path::Path::new(&repo)).map(|p| p.display().to_string());
                        store.put_repo_icon(&repo, found.as_deref());
                        found
                    }
                    _ => None,
                }?;
                let data_url = mehen_core::icons::data_url(std::path::Path::new(&path))?;
                Some(RepoIcon { repo, data_url })
            })
            .collect()
    })
    .await
    .map_err(err)
}

/// Packages kept on a release line.
#[tauri::command]
fn holds(state: State<'_, AppState>) -> Vec<Hold> {
    state.store.holds()
}

/// Keeps `name` on `line` (`5` for 5.x) in `scope`: a project folder, or `*` for every project.
/// Takes effect at the next check.
#[tauri::command]
fn set_hold(state: State<'_, AppState>, ecosystem: Ecosystem, name: String, scope: String, line: String) -> Result<Vec<Hold>, String> {
    state.store.put_hold(ecosystem, &name, &scope, &line).map_err(err)?;
    Ok(state.store.holds())
}

#[tauri::command]
fn remove_hold(state: State<'_, AppState>, id: i64) -> Result<Vec<Hold>, String> {
    state.store.remove_hold(id).map_err(err)?;
    Ok(state.store.holds())
}

/// Commits already-applied updates, one commit per repository.
#[tauri::command]
fn commit_update(plans: Vec<UpdatePlan>, force: Option<bool>) -> Vec<CommitOutcome> {
    batch::commit(plans, force.unwrap_or(false))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchResult {
    outcomes: Vec<JobOutcome>,
    /// A fresh check when anything was updated (served from the cache).
    inventory: Option<Inventory>,
}

/// Applies reviewed plans for many projects: one job per repository, running
/// side by side except where two need the same tool. Progress arrives as
/// `mehen://batch` events; results are re-checked once at the end.
#[tauri::command]
async fn apply_batch(app: AppHandle, plans: Vec<UpdatePlan>, build: bool, test: bool, commit: bool, stop_on_failure: Option<bool>, force_commit: Option<bool>) -> Result<BatchResult, String> {
    let parallel = app.state::<AppState>().update_parallel();
    let options = BatchOptions { build, test, commit, force_commit: force_commit.unwrap_or(false), parallel, stop_on_failure: stop_on_failure.unwrap_or(true) };
    let cancels = Arc::new(Cancels::default());
    *app.state::<AppState>().cancels.lock().unwrap_or_else(|e| e.into_inner()) = cancels.clone();
    let outcomes = batch::run(plans, options, &cancels, |step, cancel| async move { update::run_step(&step, &cancel).await }, |e: BatchEvent| {
        let _ = app.emit("mehen://batch", e);
    })
    .await;
    let inventory = if outcomes.iter().any(|o| o.ok) { Some(check_now(&app, false, None).await?) } else { None };
    Ok(BatchResult { outcomes, inventory })
}

/// A package's published versions for one project: whether each fits and
/// what it asks for.
#[tauri::command]
async fn package_versions(app: AppHandle, project_id: String, name: String) -> Result<Vec<check::VersionView>, String> {
    let state = app.state::<AppState>();
    let project = state.project(&project_id)?;
    check::project_context(&state.store, &project).await.versions(&name).ok_or_else(|| format!("Mehen has no version list for {name} yet. Check again first."))
}

/// What else has to move for `name` to go to `version` in one project.
#[tauri::command]
async fn move_with(app: AppHandle, project_id: String, name: String, version: String) -> Result<Vec<check::Move>, String> {
    let state = app.state::<AppState>();
    let project = state.project(&project_id)?;
    check::project_context(&state.store, &project).await.move_with(&name, &version)
}

#[tauri::command]
async fn release_dates(ecosystem: Ecosystem, name: String) -> Result<HashMap<String, String>, String> {
    mehen_core::registry::release_dates(&mehen_core::registry::http_client(), ecosystem, &name).await.map_err(|e| format!("{e:#}"))
}

/// Stops one repository's update (`job`), or all of them.
#[tauri::command]
fn cancel_update(state: State<'_, AppState>, job: Option<String>) {
    state.cancels.lock().unwrap_or_else(|e| e.into_inner()).cancel(job.as_deref());
}

#[tauri::command]
fn store_stats(state: State<'_, AppState>) -> Result<StoreStats, String> {
    state.store.stats().map_err(err)
}

#[tauri::command]
fn clear_cache(state: State<'_, AppState>) -> Result<(), String> {
    state.store.clear_cache().map_err(err)
}

/// Opens a folder in VS Code (`code` on PATH).
#[tauri::command]
fn open_in_editor(path: String) -> Result<(), String> {
    // A Mac install rarely has `code` on PATH (it is an opt-in menu command in
    // VS Code), but the app itself is always findable by its bundle id.
    #[cfg(target_os = "macos")]
    {
        let opened = std::process::Command::new("open").args(["-b", "com.microsoft.VSCode"]).arg(&path).status().map_err(err)?;
        return if opened.success() { Ok(()) } else { Err("Visual Studio Code is not installed".into()) };
    }
    #[cfg(not(target_os = "macos"))]
    open_in_editor_with_code(path)
}

#[cfg(not(target_os = "macos"))]
fn open_in_editor_with_code(path: String) -> Result<(), String> {
    let mut cmd = std::process::Command::new(if cfg!(windows) { "cmd" } else { "code" });
    if cfg!(windows) {
        cmd.args(["/C", "code", &path]);
    } else {
        cmd.arg(&path);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.spawn().map(|_| ()).map_err(err)
}

/// Project folders set up in GitWyrm that Mehen could check too.
#[tauri::command]
fn gitwyrm_folders(state: State<'_, AppState>) -> Vec<String> {
    gitwyrm::code_folders(&state.store.folders())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "macos")]
    macos_env::adopt_login_shell_path();
    // Lives until the app exits, so a panic's report is sent before it goes.
    let _telemetry = telemetry::init();
    tauri::Builder::default()
        // Registered first, as the plugin requires. A second launch (GitWyrm
        // asking to show a repository, say) hands its folder to this window
        // instead of starting another Mehen.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(request) = gitwyrm::request_from_args(argv) {
                let _ = app.emit("mehen://open-repo", request);
            }
            show_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(self_update::Pending::default())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            // Before the database opens: SQLite only takes its log callback then.
            mehen_core::trace::init(&dir, "app");
            let (store, repaired) = Store::open_or_repair(&dir.join("mehen.db")).map_err(|e| e.to_string())?;
            let inventory = std::sync::Mutex::new(store.last_inventory());
            app.manage(AppState { store, checking: AtomicBool::new(false), cancels: Default::default(), inventory });
            telemetry::set_enabled(app.state::<AppState>().error_reports());
            if let Some(note) = repaired {
                eprintln!("{note}");
                report_damage(&note);
            }
            build_tray(app)?;
            start_background_loop(app.handle().clone());
            // Re-created on every start, so the task follows Mehen when it is
            // updated or moved. Off the main thread: it runs schtasks.
            if app.state::<AppState>().scheduled_check() {
                std::thread::spawn(|| {
                    if let Err(e) = background::set_scheduled(true) {
                        eprintln!("{e}");
                    }
                });
            }
            gitwyrm::set_pending(gitwyrm::request_from_args(std::env::args()));
            Ok(())
        })
        .on_window_event(|window, event| {
            // With background checks on, closing the window keeps Mehen in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.app_handle().state::<AppState>().background_hours() > 0 {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            settings,
            add_folder,
            remove_folder,
            set_background_hours,
            set_update_parallel,
            set_notify,
            set_app_update_check,
            set_scheduled_check,
            set_error_reports,
            add_ignore,
            remove_ignore,
            discover,
            last_inventory,
            scan_and_check,
            plan_update,
            apply_batch,
            cancel_update,
            check_commands,
            set_check_commands,
            commit_update,
            repo_icons,
            holds,
            set_hold,
            remove_hold,
            version_policy,
            set_version_policy,
            store_stats,
            package_versions,
            move_with,
            release_dates,
            clear_cache,
            open_in_editor,
            gitwyrm_folders,
            gitwyrm::gitwyrm_installed,
            gitwyrm::open_in_gitwyrm,
            gitwyrm::launch_repo,
            self_update::check_self_update,
            self_update::download_self_update,
            self_update::install_self_update,
            self_update::self_update_notes
        ])
        .run(tauri::generate_context!())
        .expect("error while running Mehen");
}
