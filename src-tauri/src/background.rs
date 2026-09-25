//! Checks that run with no window.
//!
//! Most people spend their day in GitWyrm, not Mehen. So Mehen can run a check
//! and exit without ever opening its window: GitWyrm starts one when Mehen's
//! answer is more than 12 hours old or a repository's packages changed, and an
//! optional Windows scheduled task starts one daily. A warm check of dozens of
//! projects takes well under a second, so nothing needs to stay running.
//!
//! These runs skip Tauri and the WebView entirely. The same pipeline serves the
//! app, so a check means the same thing wherever it starts.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mehen_core::status::{self, Fresh};
use mehen_core::{CheckOptions, IgnoreSet, Inventory, Progress, Store, path_within};

/// Runs one check with no window, then exits.
pub const FLAG: &str = "--background-check";
/// Checks only this folder; may be given more than once.
const ONLY_FLAG: &str = "--only";
/// Started by the scheduled task: may notify about new fixes.
const SCHEDULED_FLAG: &str = "--scheduled";

/// Setting key: `true` when the daily scheduled task should exist.
pub const SCHEDULED_CHECK: &str = "scheduled_check";
/// Setting key: the day (days since 1970) the last new-fix notification was shown.
const DIGEST_DAY: &str = "scheduled_digest_day";
/// Setting key: `false` turns off notifications about new vulnerabilities.
const NOTIFY: &str = "notify_vulnerabilities";
/// The scheduled task's name. Top level: creating a task folder needs admin rights.
const TASK_NAME: &str = "Mehen background check";

pub fn requested(args: &[String]) -> bool {
    args.iter().any(|a| a == FLAG)
}

/// Mehen's data folder, resolved the way Tauri resolves it for Mehen's
/// identifier, for runs that never start Tauri.
pub fn data_dir() -> Option<PathBuf> {
    const IDENTIFIER: &str = "dev.mehen.app";
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    Some(base?.join(IDENTIFIER))
}

pub fn written_by() -> String {
    format!("Mehen {}", env!("CARGO_PKG_VERSION"))
}

/// Held for the length of a check, across processes: the open app and a
/// background run never check at the same time. Released when dropped.
pub struct CheckLock(#[allow(dead_code)] File);

impl CheckLock {
    pub fn try_take(dir: &Path) -> Option<Self> {
        std::fs::create_dir_all(dir).ok()?;
        let file = File::create(dir.join("check.lock")).ok()?;
        file.try_lock().ok()?;
        Some(Self(file))
    }

    /// Waits for a background run to finish. They take about a second, so a
    /// short wait beats telling the person to try again.
    pub async fn take(dir: &Path, wait: Duration) -> Option<Self> {
        let deadline = std::time::Instant::now() + wait;
        loop {
            if let Some(lock) = Self::try_take(dir) {
                return Some(lock);
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

/// Rewrites the per-repository summary other apps read, such as GitWyrm's
/// dependency status. Failing to write it never fails the check.
pub fn publish(dir: &Path, inventory: &Inventory, fresh: Fresh) {
    let exe = std::env::current_exe().ok().map(|p| p.display().to_string());
    if let Err(e) = status::write(dir, inventory, fresh, &written_by(), exe.as_deref()) {
        eprintln!("Writing {} failed: {e:#}", status::FILE_NAME);
    }
}

/// Scans the watched folders (minus ignored ones), checks every package,
/// saves the result and publishes the summary. With `only`, checks just those
/// folders and folds the result into the last one; folders outside every
/// watched folder are left alone. The caller holds the [`CheckLock`] and
/// prunes the cache after a full check.
pub async fn run_check(store: &Store, dir: &Path, refresh: bool, only: Option<Vec<String>>, emit: impl Fn(Progress)) -> Result<Inventory, String> {
    emit(Progress { phase: "Finding projects".into(), done: 0, total: 0 });
    let folders = store.folders();
    if folders.is_empty() {
        return Err("Add a folder with your projects first".into());
    }
    let only = only.map(|paths| paths.into_iter().filter(|p| folders.iter().any(|f| path_within(p, f))).collect::<Vec<_>>());
    if only.as_ref().is_some_and(Vec::is_empty) {
        return Err("That folder is not in a folder Mehen checks".into());
    }
    let roots: Vec<PathBuf> = folders.iter().map(PathBuf::from).collect();
    let ignore = IgnoreSet::new(&store.ignore_rules(), &roots);
    let previous = only.as_ref().and_then(|_| store.last_inventory());
    let scan_roots: Vec<PathBuf> = match &only {
        Some(paths) => paths.iter().map(PathBuf::from).collect(),
        None => roots,
    };
    let inventory = tokio::task::spawn_blocking(move || mehen_core::scan(&scan_roots, &ignore)).await.map_err(|e| e.to_string())?;
    let options = if refresh { CheckOptions::refresh() } else { CheckOptions::default() };
    let checked = mehen_core::check(inventory, store, options, emit).await;
    match (only, previous) {
        (Some(paths), Some(previous)) => {
            let merged = previous.merge_partial(checked, &paths);
            store.replace_last_inventory(&merged).map_err(|e| e.to_string())?;
            publish(dir, &merged, Fresh::Only(&paths));
            Ok(merged)
        }
        (Some(paths), None) => {
            publish(dir, &checked, Fresh::Only(&paths));
            Ok(checked)
        }
        (None, _) => {
            publish(dir, &checked, Fresh::All);
            Ok(checked)
        }
    }
}

fn only_paths(args: &[String]) -> Option<Vec<String>> {
    let paths: Vec<String> = args.windows(2).filter(|w| w[0] == ONLY_FLAG).map(|w| w[1].clone()).collect();
    (!paths.is_empty()).then_some(paths)
}

/// Runs one check with no window and returns the process exit code. Quietly
/// does nothing when another check is running or no folders are set up.
pub fn run(args: &[String]) -> i32 {
    let Some(dir) = data_dir() else { return 1 };
    let Some(_lock) = CheckLock::try_take(&dir) else { return 0 };
    let Ok(store) = Store::open(&dir.join("mehen.db")) else { return 1 };
    if store.folders().is_empty() {
        return 0;
    }
    let only = only_paths(args);
    let before = status::read(&dir);
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return 1 };
    // No cache cleanup here: it rewrites the database's free-page lists, so it
    // stays with the app, one process, under the check lock.
    if runtime.block_on(run_check(&store, &dir, false, only, |_| {})).is_err() {
        return 0;
    }
    if args.iter().any(|a| a == SCHEDULED_FLAG) && store.setting(NOTIFY).is_none_or(|v| v != "false") {
        if let (Some(before), Some(after)) = (before, status::read(&dir)) {
            notify_new_fixes(&store, &before, &after);
        }
    }
    0
}

/// Security problems that have a fix, one key each, per repository.
fn fix_keys(file: &status::StatusFile) -> Vec<(String, String)> {
    file.repos
        .iter()
        .flat_map(|r| {
            r.problems.iter().filter_map(move |p| {
                let fix = p.fixed_in.as_deref()?;
                Some((r.path.clone(), format!("{}|{}:{}@{fix}", r.path.to_lowercase(), p.ecosystem, p.name)))
            })
        })
        .collect()
}

/// Repositories with fixes that were not in `before`: the only news worth a
/// notification. A problem nobody can fix yet is left for the app to show.
pub fn new_fixes(before: &status::StatusFile, after: &status::StatusFile) -> Vec<(String, usize)> {
    let old: std::collections::HashSet<String> = fix_keys(before).into_iter().map(|(_, k)| k).collect();
    let mut repos: Vec<(String, usize)> = Vec::new();
    for (repo, key) in fix_keys(after) {
        if old.contains(&key) {
            continue;
        }
        match repos.iter_mut().find(|(r, _)| *r == repo) {
            Some((_, n)) => *n += 1,
            None => repos.push((repo, 1)),
        }
    }
    repos
}

/// At most one notification a day, and only for fixes that are new.
fn notify_new_fixes(store: &Store, before: &status::StatusFile, after: &status::StatusFile) {
    let repos = new_fixes(before, after);
    if repos.is_empty() {
        return;
    }
    let today = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) / 86_400).to_string();
    if store.setting(DIGEST_DAY).as_deref() == Some(today.as_str()) {
        return;
    }
    let total: usize = repos.iter().map(|(_, n)| n).sum();
    let title = if total == 1 { "Mehen: a new security fix is ready".to_string() } else { format!("Mehen: {total} new security fixes are ready") };
    let name = |path: &str| path.rsplit(['\\', '/']).next().unwrap_or(path).to_string();
    let body = repos.iter().take(3).map(|(r, n)| if *n == 1 { name(r) } else { format!("{} ({n})", name(r)) }).collect::<Vec<_>>().join(", ");
    if show_toast(&title, &format!("For {body}. Open Mehen to update.")) {
        let _ = store.set_setting(DIGEST_DAY, &today);
    }
}

#[cfg(windows)]
fn show_toast(title: &str, body: &str) -> bool {
    use tauri_winrt_notification::Toast;
    // The installed app registers its identifier with Windows, so clicking the
    // notification opens Mehen. A development build borrows PowerShell's.
    let installed = std::env::current_exe().ok().is_some_and(|p| !p.components().any(|c| c.as_os_str() == "target"));
    let app_id = if installed { "dev.mehen.app" } else { Toast::POWERSHELL_APP_ID };
    Toast::new(app_id).title(title).text1(body).show().is_ok()
}

#[cfg(not(windows))]
fn show_toast(_title: &str, _body: &str) -> bool {
    false
}

/// Whether this computer can run the daily check without Mehen open.
pub fn schedule_supported() -> bool {
    cfg!(windows)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The scheduled task: at sign-in (after a few minutes, so sign-in stays
/// quick) and once a day, caught up later if the computer was off. Skipped on
/// battery and without a network, at below-normal priority.
fn task_xml(exe: &str, user: &str, logon: bool) -> String {
    let logon_trigger = if logon {
        format!("<LogonTrigger><Enabled>true</Enabled><UserId>{}</UserId><Delay>PT5M</Delay></LogonTrigger>", xml_escape(user))
    } else {
        String::new()
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>Checks your projects' packages for known security problems once a day. Mehen does not stay open. Turn it off in Mehen's settings.</Description></RegistrationInfo>
  <Triggers>{logon_trigger}<CalendarTrigger><StartBoundary>2026-01-01T12:00:00</StartBoundary><Enabled>true</Enabled><ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>true</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>true</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>true</RunOnlyIfNetworkAvailable>
    <IdleSettings><StopOnIdleEnd>false</StopOnIdleEnd><RestartOnIdle>false</RestartOnIdle></IdleSettings>
    <ExecutionTimeLimit>PT15M</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author"><Exec><Command>{exe}</Command><Arguments>{FLAG} {SCHEDULED_FLAG}</Arguments></Exec></Actions>
</Task>
"#,
        user = xml_escape(user),
        exe = xml_escape(exe),
    )
}

#[cfg(windows)]
fn schtasks(args: &[&str]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("schtasks").args(args).creation_flags(0x0800_0000).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Creates (or replaces) the daily task pointing at this copy of Mehen, or
/// removes it. Replacing is how a moved or updated Mehen keeps it working.
#[cfg(windows)]
pub fn set_scheduled(enabled: bool) -> Result<(), String> {
    if !enabled {
        // Already gone is fine: that is the state being asked for.
        let _ = schtasks(&["/Delete", "/TN", TASK_NAME, "/F"]);
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?.display().to_string();
    let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
        (Ok(domain), Ok(name)) => format!("{domain}\\{name}"),
        (_, Ok(name)) => name,
        _ => return Err("Could not tell which Windows account to run the check as".into()),
    };
    let file = std::env::temp_dir().join("mehen-background-check.xml");
    let create = |logon: bool| -> Result<(), String> {
        let xml: Vec<u16> = std::iter::once(0xFEFF).chain(task_xml(&exe, &user, logon).encode_utf16()).collect();
        let bytes: Vec<u8> = xml.iter().flat_map(|u| u.to_le_bytes()).collect();
        std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
        let path = file.display().to_string();
        schtasks(&["/Create", "/TN", TASK_NAME, "/XML", &path, "/F"])
    };
    // Some Windows setups refuse a sign-in trigger without admin rights; the
    // daily run, caught up when the computer comes back on, still covers it.
    let result = create(true).or_else(|_| create(false));
    let _ = std::fs::remove_file(&file);
    result.map_err(|e| format!("Windows would not create the daily check: {e}"))
}

#[cfg(not(windows))]
pub fn set_scheduled(_enabled: bool) -> Result<(), String> {
    Err("Daily checks without Mehen open are only available on Windows for now".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(repos: Vec<(&str, Vec<(&str, Option<&str>)>)>) -> status::StatusFile {
        let json = serde_json::json!({
            "format": 1, "writtenBy": "t", "exe": null, "writtenAt": 0,
            "repos": repos.into_iter().map(|(path, problems)| serde_json::json!({
                "path": path, "checkedAt": 0, "checkedCommit": null, "vulnerable": problems.len(), "fixable": 0,
                "severity": {"critical":0,"high":0,"moderate":0,"low":0,"unknown":0}, "outdated": 0,
                "problems": problems.into_iter().map(|(name, fix)| serde_json::json!({
                    "name": name, "ecosystem": "npm", "version": "1.0.0", "fixedIn": fix, "severity": "HIGH", "summary": "", "advisory": "X", "url": ""
                })).collect::<Vec<_>>()
            })).collect::<Vec<_>>()
        });
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn only_new_fixes_are_news() {
        let before = file(vec![("C:\\code\\a", vec![("old", Some("1.0.1"))])]);
        let after = file(vec![
            ("C:\\code\\a", vec![("old", Some("1.0.1")), ("fresh", Some("2.0.0")), ("stuck", None)]),
            ("C:\\code\\b", vec![("x", Some("1.1.0")), ("y", Some("3.0.0"))]),
        ]);
        assert_eq!(new_fixes(&before, &after), vec![("C:\\code\\a".to_string(), 1), ("C:\\code\\b".to_string(), 2)]);
        assert!(new_fixes(&after, &after).is_empty());
    }

    #[test]
    fn reads_every_only_folder() {
        let args: Vec<String> = ["mehen.exe", FLAG, ONLY_FLAG, "C:\\a", ONLY_FLAG, "C:\\b"].iter().map(|s| s.to_string()).collect();
        assert_eq!(only_paths(&args), Some(vec!["C:\\a".to_string(), "C:\\b".to_string()]));
        assert_eq!(only_paths(&args[..2]), None);
        assert!(requested(&args));
    }

    #[test]
    fn the_task_runs_this_copy_of_mehen_in_the_background() {
        let xml = task_xml("C:\\Apps & Tools\\mehen.exe", "PC\\me", true);
        assert!(xml.contains("<Command>C:\\Apps &amp; Tools\\mehen.exe</Command>"));
        assert!(xml.contains("<Arguments>--background-check --scheduled</Arguments>"));
        assert!(xml.contains("<LogonTrigger>"));
        assert!(!task_xml("m.exe", "me", false).contains("<LogonTrigger>"));
    }

    /// Touches this computer's Task Scheduler: run by hand with `--ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn windows_accepts_the_task_without_admin_rights() {
        set_scheduled(true).expect("create");
        let out = std::process::Command::new("schtasks").args(["/Query", "/TN", TASK_NAME, "/XML"]).output().unwrap();
        let xml = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success());
        println!("sign-in trigger kept: {}", xml.contains("LogonTrigger"));
        set_scheduled(false).expect("remove");
        assert!(schtasks(&["/Query", "/TN", TASK_NAME]).is_err());
    }

    #[test]
    fn a_second_check_waits_its_turn() {
        let dir = std::env::temp_dir().join("mehen-check-lock");
        let first = CheckLock::try_take(&dir).expect("first lock");
        assert!(CheckLock::try_take(&dir).is_none());
        drop(first);
        assert!(CheckLock::try_take(&dir).is_some());
    }
}
