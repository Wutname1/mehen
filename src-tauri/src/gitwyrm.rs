//! Working alongside GitWyrm, the git app: opening a repository in it,
//! offering its project folders on first run, and showing the repository
//! GitWyrm (or anyone) passes on the command line.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// With a folder: select that repository's security fixes, ready to update.
const FIX_FLAG: &str = "--fix";

/// A repository someone asked Mehen to show.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenRequest {
    pub path: String,
    /// Select its security fixes too, so updating is one click away.
    pub fix: bool,
}

/// A request passed at launch, held until the window asks for it.
static PENDING: Mutex<Option<OpenRequest>> = Mutex::new(None);

/// The first argument that is an existing folder, and whether `--fix` came
/// with it. Other flags are skipped, and so is argv[0], which is Mehen itself.
pub fn request_from_args<I, S>(args: I) -> Option<OpenRequest>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args.into_iter().skip(1).map(|a| a.as_ref().to_string()).collect();
    let path = args.iter().find(|a| !a.starts_with('-') && Path::new(a).is_dir())?.clone();
    Some(OpenRequest { path, fix: args.iter().any(|a| a == FIX_FLAG) })
}

pub fn set_pending(request: Option<OpenRequest>) {
    if let (Some(request), Ok(mut slot)) = (request, PENDING.lock()) {
        *slot = Some(request);
    }
}

/// What Mehen was started to show, once. Taken rather than read so a reload
/// of the window does not jump back to it.
#[tauri::command]
pub fn launch_repo() -> Option<OpenRequest> {
    PENDING.lock().ok()?.take()
}

/// GitWyrm's program, from the entry its installer leaves for Windows'
/// installed-apps list.
#[cfg(windows)]
fn installed_exe() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\GitWyrm").ok()?;
    let location: String = key.get_value("InstallLocation").ok()?;
    let binary: String = key.get_value("MainBinaryName").unwrap_or_else(|_| "gitwyrm.exe".into());
    let exe = PathBuf::from(location.trim_matches('"')).join(binary);
    exe.is_file().then_some(exe)
}

#[cfg(not(windows))]
fn installed_exe() -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).map(|dir| dir.join("gitwyrm")).find(|p| p.is_file())
}

#[tauri::command]
pub fn gitwyrm_installed() -> bool {
    installed_exe().is_some()
}

/// Opens `path` as a tab in GitWyrm. A GitWyrm that is already running takes
/// the folder into its own window instead of starting again.
#[tauri::command]
pub fn open_in_gitwyrm(path: String) -> Result<(), String> {
    let exe = installed_exe().ok_or("GitWyrm is not installed")?;
    std::process::Command::new(exe).arg(&path).spawn().map(|_| ()).map_err(|e| format!("Could not start GitWyrm: {e}"))
}

/// GitWyrm's data folder, resolved the way Tauri resolves it for GitWyrm's
/// identifier. If GitWyrm's identifier ever changes, this must follow.
fn gitwyrm_data_dir() -> Option<PathBuf> {
    const IDENTIFIER: &str = "dev.gitwyrm.app";
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    Some(base?.join(IDENTIFIER))
}

#[derive(Deserialize)]
struct GitWyrmSettings {
    #[serde(default)]
    code_folders: Vec<CodeFolder>,
    /// The single folder older versions kept.
    #[serde(default)]
    code_folder: Option<String>,
}

#[derive(Deserialize)]
struct CodeFolder {
    path: String,
}

/// The project folders set up in GitWyrm that exist and Mehen does not watch
/// yet. Anything unreadable simply means there is nothing to offer.
pub fn code_folders(watched: &[String]) -> Vec<String> {
    let Some(raw) = gitwyrm_data_dir().and_then(|dir| std::fs::read_to_string(dir.join("settings.json")).ok()) else {
        return Vec::new();
    };
    let Ok(settings) = serde_json::from_str::<GitWyrmSettings>(&raw) else {
        return Vec::new();
    };
    let mut folders: Vec<String> = settings.code_folders.into_iter().map(|f| f.path).collect();
    if folders.is_empty() {
        folders.extend(settings.code_folder);
    }
    let norm = |p: &str| p.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let mut seen: Vec<String> = watched.iter().map(|w| norm(w)).collect();
    folders
        .into_iter()
        .filter(|f| {
            let key = norm(f);
            let fresh = Path::new(f).is_dir() && !seen.contains(&key);
            seen.push(key);
            fresh
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_first_folder_argument() {
        let dir = std::env::temp_dir().display().to_string();
        assert_eq!(request_from_args(["mehen.exe", "--flag", "not-a-folder-xyz", dir.as_str()]), Some(OpenRequest { path: dir.clone(), fix: false }));
        assert_eq!(request_from_args(["mehen.exe", dir.as_str(), "--fix"]), Some(OpenRequest { path: dir.clone(), fix: true }));
        assert_eq!(request_from_args(["mehen.exe"]), None);
        assert_eq!(request_from_args([dir.as_str()]), None);
    }
}
