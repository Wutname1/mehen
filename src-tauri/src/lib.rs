use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mehen_core::batch::{self, BatchEvent, BatchOptions, CommitOutcome, JobOutcome};
use mehen_core::store::StoreStats;
use mehen_core::update::{self, Change, UpdatePlan};
use mehen_core::{CheckOptions, DiscoveredProject, Ecosystem, IgnoreKind, IgnoreRule, IgnoreSet, Inventory, Progress, Store};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_notification::NotificationExt;

/// Setting key: hours between background checks; 0 or missing means off.
const BACKGROUND_HOURS: &str = "background_hours";
/// Setting key: how many update steps may run at once across repositories.
const UPDATE_PARALLEL: &str = "update_parallel";
/// Setting key: JSON map of scope to the commands that replace a project's
/// build and test steps. A scope is a repository path, or `ecosystem:<name>`
/// for the default of every project of that kind.
const CHECK_COMMANDS: &str = "check_commands";
/// How often the background loop wakes to see whether a check is due.
const BACKGROUND_TICK: Duration = Duration::from_secs(10 * 60);

struct AppState {
    store: Store,
    /// Set while a check runs, so background and manual checks never overlap.
    checking: AtomicBool,
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
    update_parallel: usize,
}

impl AppState {
    fn settings(&self) -> Settings {
        Settings { folders: self.store.folders(), rules: self.store.ignore_rules(), background_hours: self.background_hours(), update_parallel: self.update_parallel() }
    }

    fn roots(&self) -> Vec<PathBuf> {
        self.store.folders().into_iter().map(PathBuf::from).collect()
    }

    fn update_parallel(&self) -> usize {
        self.store.setting(UPDATE_PARALLEL).and_then(|v| v.parse().ok()).unwrap_or(batch::DEFAULT_PARALLEL).clamp(1, 8)
    }

    fn check_commands(&self) -> BTreeMap<String, Vec<String>> {
        self.store.setting(CHECK_COMMANDS).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    fn background_hours(&self) -> u32 {
        self.store.setting(BACKGROUND_HOURS).and_then(|v| v.parse().ok()).unwrap_or(0)
    }
}

/// Scans the watched folders (minus ignored ones) and checks every package.
/// With `only`, scans just those folders and folds the result into the last one.
async fn check_now(app: &AppHandle, refresh: bool, only: Option<Vec<String>>) -> Result<Inventory, String> {
    let state = app.state::<AppState>();
    let emit = |p: Progress| {
        let _ = app.emit("mehen://progress", p);
    };
    emit(Progress { phase: "Finding projects".into(), done: 0, total: 0 });
    let roots = state.roots();
    if roots.is_empty() {
        return Err("Add a folder to watch first".into());
    }
    let ignore = IgnoreSet::new(&state.store.ignore_rules(), &roots);
    let previous = only.as_ref().and_then(|_| state.store.last_inventory());
    let scan_roots: Vec<PathBuf> = match &only {
        Some(folders) => folders.iter().map(PathBuf::from).collect(),
        None => roots,
    };
    let inventory = tauri::async_runtime::spawn_blocking(move || mehen_core::scan(&scan_roots, &ignore)).await.map_err(err)?;
    let options = if refresh { CheckOptions::refresh() } else { CheckOptions::default() };
    let checked = mehen_core::check(inventory, &state.store, options, emit).await;
    match (only, previous) {
        (Some(folders), Some(previous)) => {
            let merged = previous.merge_partial(checked, &folders);
            state.store.replace_last_inventory(&merged).map_err(err)?;
            Ok(merged)
        }
        _ => Ok(checked),
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

/// How many update steps may run at once; 1 runs one at a time.
#[tauri::command]
fn set_update_parallel(state: State<'_, AppState>, parallel: usize) -> Result<Settings, String> {
    state.store.set_setting(UPDATE_PARALLEL, &parallel.clamp(1, 8).to_string()).map_err(err)?;
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
fn add_ignore(state: State<'_, AppState>, kind: IgnoreKind, value: String, note: Option<String>) -> Result<IgnoreResult, String> {
    state.store.add_ignore_rule(kind, &value, note.as_deref()).map_err(err)?;
    let inventory = state.store.last_inventory().map(|mut inv| {
        IgnoreSet::new(&state.store.ignore_rules(), &state.roots()).apply(&mut inv);
        inv
    });
    if let Some(inv) = &inventory {
        state.store.replace_last_inventory(inv).map_err(err)?;
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
    state.store.last_inventory()
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
    let inventory = state.store.last_inventory().ok_or("Run a check first")?;
    let project = inventory.projects.iter().find(|p| p.id == project_id).ok_or("That project is not in the last check")?;
    let mut plan = update::plan(project, &changes, |name| state.store.package_any_age(Ecosystem::GithubActions, name)).map_err(|e| format!("{e:#}"))?;
    let commands = state.check_commands();
    let repo = project.repo.clone().unwrap_or_else(|| project.dir.clone());
    if let Some(list) = commands.iter().find(|(scope, _)| scope.eq_ignore_ascii_case(&repo)).map(|(_, list)| list) {
        update::use_check_commands(&mut plan, list, &repo);
    } else if let Some(list) = commands.get(&format!("ecosystem:{}", project.ecosystem.key())) {
        update::use_check_commands(&mut plan, list, &project.dir);
    }
    Ok(plan)
}

/// Build and test commands the user set, by scope.
#[tauri::command]
fn check_commands(state: State<'_, AppState>) -> BTreeMap<String, Vec<String>> {
    state.check_commands()
}

/// Sets (or with `None`, removes) the commands for a scope.
#[tauri::command]
fn set_check_commands(state: State<'_, AppState>, scope: String, commands: Option<Vec<String>>) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut all = state.check_commands();
    all.retain(|k, _| !k.eq_ignore_ascii_case(&scope));
    if let Some(list) = commands {
        all.insert(scope, list.into_iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect());
    }
    state.store.set_setting(CHECK_COMMANDS, &serde_json::to_string(&all).map_err(err)?).map_err(err)?;
    Ok(all)
}

/// Commits already-applied updates, one commit per repository.
#[tauri::command]
fn commit_update(plans: Vec<UpdatePlan>) -> Vec<CommitOutcome> {
    batch::commit(plans)
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
async fn apply_batch(app: AppHandle, plans: Vec<UpdatePlan>, checks: bool, commit: bool) -> Result<BatchResult, String> {
    let parallel = app.state::<AppState>().update_parallel();
    let options = BatchOptions { checks, commit, parallel };
    let outcomes = batch::run(plans, options, |step| async move { update::run_step(&step).await }, |e: BatchEvent| {
        let _ = app.emit("mehen://batch", e);
    })
    .await;
    let inventory = if outcomes.iter().any(|o| o.ok) { Some(check_now(&app, false, None).await?) } else { None };
    Ok(BatchResult { outcomes, inventory })
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let db = app.path().app_data_dir()?.join("mehen.db");
            let store = Store::open(&db).map_err(|e| e.to_string())?;
            app.manage(AppState { store, checking: AtomicBool::new(false) });
            build_tray(app)?;
            start_background_loop(app.handle().clone());
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
            add_ignore,
            remove_ignore,
            discover,
            last_inventory,
            scan_and_check,
            plan_update,
            apply_batch,
            check_commands,
            set_check_commands,
            commit_update,
            store_stats,
            clear_cache,
            open_in_editor
        ])
        .run(tauri::generate_context!())
        .expect("error while running Mehen");
}
