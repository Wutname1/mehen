use std::path::PathBuf;

use mehen_core::store::StoreStats;
use mehen_core::{CheckOptions, Inventory, Progress, Store};
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    store: Store,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The last saved result for a folder, so the app opens with data instead of a spinner.
#[tauri::command]
fn last_inventory(state: State<'_, AppState>, root: String) -> Option<Inventory> {
    state.store.last_inventory(&root)
}

/// Scans the folder, then checks every package. With `refresh`, cached
/// registry and vulnerability answers are ignored.
#[tauri::command]
async fn scan_and_check(app: AppHandle, state: State<'_, AppState>, root: String, refresh: bool) -> Result<Inventory, String> {
    let emit = |p: Progress| {
        let _ = app.emit("mehen://progress", p);
    };
    emit(Progress { phase: "Finding projects".into(), done: 0, total: 0 });
    let path = PathBuf::from(&root);
    let inventory = tauri::async_runtime::spawn_blocking(move || mehen_core::scan(&path)).await.map_err(err)?;
    let options = if refresh { CheckOptions::refresh() } else { CheckOptions::default() };
    Ok(mehen_core::check(inventory, &state.store, options, emit).await)
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
        .invoke_handler(tauri::generate_handler![last_inventory, scan_and_check, store_stats, clear_cache, open_in_editor])
        .run(tauri::generate_context!())
        .expect("error while running Mehen");
}
