use std::path::PathBuf;

use mehen_core::store::StoreStats;
use mehen_core::update::{self, Change, UpdateEvent, UpdateOutcome, UpdatePlan};
use mehen_core::{CheckOptions, DiscoveredProject, Ecosystem, IgnoreKind, IgnoreRule, IgnoreSet, Inventory, Progress, Store};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    store: Store,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    folders: Vec<String>,
    rules: Vec<IgnoreRule>,
}

impl AppState {
    fn settings(&self) -> Settings {
        Settings { folders: self.store.folders(), rules: self.store.ignore_rules() }
    }

    fn roots(&self) -> Vec<PathBuf> {
        self.store.folders().into_iter().map(PathBuf::from).collect()
    }
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

/// Scans the watched folders (minus ignored ones), then checks every package.
/// With `refresh`, cached registry and vulnerability answers are ignored.
#[tauri::command]
async fn scan_and_check(app: AppHandle, state: State<'_, AppState>, refresh: bool) -> Result<Inventory, String> {
    let emit = |p: Progress| {
        let _ = app.emit("mehen://progress", p);
    };
    emit(Progress { phase: "Finding projects".into(), done: 0, total: 0 });
    let roots = state.roots();
    if roots.is_empty() {
        return Err("Add a folder to watch first".into());
    }
    let ignore = IgnoreSet::new(&state.store.ignore_rules(), &roots);
    let inventory = tauri::async_runtime::spawn_blocking(move || mehen_core::scan(&roots, &ignore)).await.map_err(err)?;
    let options = if refresh { CheckOptions::refresh() } else { CheckOptions::default() };
    Ok(mehen_core::check(inventory, &state.store, options, emit).await)
}

/// Works out exactly what an update would change, without writing anything.
#[tauri::command]
fn plan_update(state: State<'_, AppState>, project_id: String, changes: Vec<Change>) -> Result<UpdatePlan, String> {
    let inventory = state.store.last_inventory().ok_or("Run a check first")?;
    let project = inventory.projects.iter().find(|p| p.id == project_id).ok_or("That project is not in the last check")?;
    update::plan(project, &changes, |name| state.store.package_any_age(Ecosystem::GithubActions, name)).map_err(|e| format!("{e:#}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApplyResult {
    outcome: UpdateOutcome,
    /// A fresh check after a successful update (served from the cache).
    inventory: Option<Inventory>,
}

/// Applies a reviewed plan, then (with `rescan`) re-checks so the results show
/// the new versions. Batch updates pass `rescan: false` and check once at the end.
#[tauri::command]
async fn apply_update(app: AppHandle, state: State<'_, AppState>, plan: UpdatePlan, verify: bool, rescan: Option<bool>) -> Result<ApplyResult, String> {
    let outcome = update::apply(&plan, verify, |e: UpdateEvent| {
        let _ = app.emit("mehen://update", e);
    })
    .await;
    if !outcome.ok || rescan == Some(false) {
        return Ok(ApplyResult { outcome, inventory: None });
    }
    let roots = state.roots();
    let ignore = IgnoreSet::new(&state.store.ignore_rules(), &roots);
    let inventory = tauri::async_runtime::spawn_blocking(move || mehen_core::scan(&roots, &ignore)).await.map_err(err)?;
    let emit = |p: Progress| {
        let _ = app.emit("mehen://progress", p);
    };
    let inventory = mehen_core::check(inventory, &state.store, CheckOptions::default(), emit).await;
    Ok(ApplyResult { outcome, inventory: Some(inventory) })
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
        .setup(|app| {
            let db = app.path().app_data_dir()?.join("mehen.db");
            let store = Store::open(&db).map_err(|e| e.to_string())?;
            app.manage(AppState { store });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            settings,
            add_folder,
            remove_folder,
            add_ignore,
            remove_ignore,
            discover,
            last_inventory,
            scan_and_check,
            plan_update,
            apply_update,
            store_stats,
            clear_cache,
            open_in_editor
        ])
        .run(tauri::generate_context!())
        .expect("error while running Mehen");
}
