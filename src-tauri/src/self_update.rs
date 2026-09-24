//! Mehen updating itself, as opposed to the dependency updates it runs on
//! projects.
//!
//! The manifest lives at the endpoint in tauri.conf.json (a static object on the
//! CDN whose download URLs point at the GitHub release), and every installer is
//! signature-checked by the updater plugin before anything runs.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

/// Event carrying download progress to the update dialog.
pub const PROGRESS_EVENT: &str = "self-update://progress";

/// Mehen's release notes. The API holds more than one product, so the filter
/// is required, not an optimisation.
const CHANGELOG_URL: &str = "https://gitwyrm.com/api/v1/changelogs?product=Mehen&channel=release&limit=100";

/// `on_chunk` fires per HTTP chunk, and every emit crosses to the webview, so
/// progress is only reported every 256 KB.
const PROGRESS_EMIT_BYTES: u64 = 256 * 1024;

#[derive(Clone, Serialize)]
pub struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

/// A downloaded, signature-checked installer waiting for the user to restart.
/// Held in memory so reading the notes or finishing a task first does not cost
/// a second download. A restart re-checks from scratch.
#[derive(Default)]
pub struct Pending(Mutex<Option<PendingInstall>>);

struct PendingInstall {
    version: String,
    bytes: Vec<u8>,
}

fn updater(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    let builder = app.updater_builder();

    // Backstop only: `install_self_update` raises the cover before it starts,
    // and the flag in `spawn_update_cover` stops a second window.
    #[cfg(windows)]
    let builder = {
        let app = app.clone();
        builder.on_before_exit(move || {
            if let Err(e) = spawn_update_cover(&app) {
                eprintln!("Update cover window did not start: {e}");
            }
        })
    };

    builder.build().map_err(|e| e.to_string())
}

/// The newer version on offer, or None when Mehen is up to date.
#[tauri::command]
pub async fn check_self_update(app: AppHandle) -> Result<Option<String>, String> {
    let update = updater(&app)?.check().await.map_err(|e| e.to_string())?;
    Ok(update.map(|u| u.version))
}

/// Downloads the update and holds it without installing, so the restart can
/// wait until the user is ready.
#[tauri::command]
pub async fn download_self_update(app: AppHandle, pending: tauri::State<'_, Pending>) -> Result<Option<String>, String> {
    let Some(update) = updater(&app)?.check().await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };

    let mut downloaded: u64 = 0;
    let mut last_emit: u64 = 0;
    let progress_app = app.clone();
    let on_chunk = move |chunk: usize, total: Option<u64>| {
        downloaded = downloaded.saturating_add(chunk as u64);
        // The final chunk always reports, so the bar lands on 100%.
        let complete = total.is_some_and(|t| downloaded >= t);
        if downloaded - last_emit < PROGRESS_EMIT_BYTES && !complete {
            return;
        }
        last_emit = downloaded;
        let _ = progress_app.emit(PROGRESS_EVENT, Progress { downloaded, total });
    };

    let bytes = update.download(on_chunk, || {}).await.map_err(|e| e.to_string())?;
    let version = update.version.clone();
    *pending.0.lock().map_err(|e| e.to_string())? = Some(PendingInstall { version: version.clone(), bytes });
    Ok(Some(version))
}

/// Installs the update `download_self_update` fetched. Does not return on
/// success: the updater exits the process once the installer is launched.
#[tauri::command]
pub async fn install_self_update(app: AppHandle, pending: tauri::State<'_, Pending>) -> Result<(), String> {
    let Some(ready) = pending.0.lock().map_err(|e| e.to_string())?.take() else {
        return Err("No Mehen update has been downloaded yet.".into());
    };

    let Some(update) = updater(&app)?.check().await.map_err(|e| e.to_string())? else {
        return Err("That Mehen update is no longer being offered.".into());
    };
    if update.version != ready.version {
        return Err(format!("Mehen {} was downloaded, but {} is now the latest. Download again to get it.", ready.version, update.version));
    }

    // The plugin's before-exit hook runs only after the installer has been
    // written to disk, seconds after the click. Covering the screen now makes
    // the handover immediate.
    #[cfg(windows)]
    match spawn_update_cover(&app) {
        Ok(()) => hide_all_windows(&app),
        Err(e) => eprintln!("Update cover window did not start: {e}"),
    }

    update.install(ready.bytes).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangelogItem {
    pub section: String,
    pub text: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct ChangelogEntry {
    pub version: String,
    pub released_at: Option<String>,
    #[serde(default)]
    pub items: Vec<ChangelogItem>,
}

#[derive(Deserialize)]
struct ChangelogResponse {
    #[serde(default)]
    entries: Vec<ChangelogEntry>,
}

/// Release notes for every version after `current` up to `target`, newest
/// first. Someone skipping several releases sees all of them, since this is
/// their only chance to read them. Fetched here so the page never needs network
/// access of its own.
#[tauri::command]
pub async fn self_update_notes(current: String, target: String) -> Result<Vec<ChangelogEntry>, String> {
    let response = reqwest::get(CHANGELOG_URL).await.map_err(|e| format!("Could not reach the release notes: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Release notes request failed: {}", response.status()));
    }
    let body: ChangelogResponse = response.json().await.map_err(|e| format!("Could not read the release notes: {e}"))?;
    Ok(notes_between(body.entries, &current, &target))
}

fn notes_between(entries: Vec<ChangelogEntry>, current: &str, target: &str) -> Vec<ChangelogEntry> {
    let (from, to) = (parse_version(current), parse_version(target));
    let mut notes: Vec<ChangelogEntry> = entries
        .into_iter()
        .filter(|e| !e.version.contains('-'))
        .filter(|e| {
            let v = parse_version(&e.version);
            v > from && v <= to
        })
        .collect();
    notes.sort_by_key(|e| std::cmp::Reverse(parse_version(&e.version)));
    notes
}

/// Lenient on purpose: an unparseable part becomes 0, so a malformed entry
/// sorts low instead of failing the whole list.
fn parse_version(v: &str) -> (u32, u32, u32) {
    let core = v.trim_start_matches('v');
    let core = core.split(['-', '+']).next().unwrap_or(core);
    let mut parts = core.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

/// Set once the cover has been started, so the up-front call and the
/// before-exit backstop never open two windows.
#[cfg(windows)]
static COVER_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Shows the "Updating Mehen" window over the gap between this process exiting
/// and the new version appearing, which is otherwise 20-40 seconds of nothing
/// while the installer runs quietly.
///
/// Runs from a temp copy because the installer is about to rewrite the install
/// folder, and detached because this process is killed moments later. Failure
/// is non-fatal: the update still installs, just without the cover.
#[cfg(windows)]
fn spawn_update_cover(app: &AppHandle) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::sync::atomic::Ordering;

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    if COVER_STARTED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    let start = || -> Result<(), String> {
        let source = app.path().resource_dir().map_err(|e| format!("no resource dir: {e}"))?.join("resources").join("mehen-setup.exe");
        if !source.is_file() {
            return Err(format!("helper missing at {}", source.display()));
        }
        let dest = std::env::temp_dir().join(format!("mehen-update-{}.exe", std::process::id()));
        std::fs::copy(&source, &dest).map_err(|e| format!("could not stage helper at {}: {e}", dest.display()))?;
        std::process::Command::new(&dest)
            .arg("--updating")
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .map_err(|e| format!("could not start helper: {e}"))?;
        Ok(())
    };

    // Release the claim on failure so the before-exit backstop still gets a try.
    start().inspect_err(|_| COVER_STARTED.store(false, Ordering::SeqCst))
}

/// Hides rather than closes: closing the last window would run the exit path
/// and end the process that still has an installer to launch.
#[cfg(windows)]
fn hide_all_windows(app: &AppHandle) {
    for window in app.webview_windows().values() {
        let _ = window.hide();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(version: &str) -> ChangelogEntry {
        ChangelogEntry { version: version.into(), released_at: None, items: vec![] }
    }

    fn versions(notes: Vec<ChangelogEntry>) -> Vec<String> {
        notes.into_iter().map(|e| e.version).collect()
    }

    #[test]
    fn notes_cover_every_skipped_release_newest_first() {
        let all = vec![entry("0.1.0"), entry("0.3.0"), entry("0.2.0"), entry("0.2.1"), entry("0.4.0")];
        assert_eq!(versions(notes_between(all, "0.1.0", "0.3.0")), ["0.3.0", "0.2.1", "0.2.0"]);
    }

    #[test]
    fn notes_skip_prereleases_and_order_numerically() {
        let all = vec![entry("0.10.0"), entry("0.9.0"), entry("0.10.0-beta.1")];
        assert_eq!(versions(notes_between(all, "0.8.0", "0.10.0")), ["0.10.0", "0.9.0"]);
    }
}
