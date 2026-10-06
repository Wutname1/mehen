//! Finding a tool on this computer and checking it is new enough to drive.
//!
//! A GUI app inherits a `PATH` from whatever started it (Explorer on Windows,
//! launchd on macOS), which can predate a tool installed since. So a tool not
//! found on our own `PATH` or in the usual install folders is looked for
//! again on the `PATH` a fresh shell would have, without changing this
//! process's environment.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::process::probe_command;
use crate::{Availability, Detected, Tool};

/// A hung binary must not hold up the screen that asked.
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
const SHELL_TIMEOUT: Duration = Duration::from_secs(5);

/// The oldest version each tool's protocol was checked against.
///
/// Claude Code: `--restricted` with named `--tools`, measured on 2.1.260;
/// the permission frames on 2.1.251 and again on 2.1.287. Codex: the
/// app-server approval vocabulary (`accept`/`decline`), from 0.151.0's own
/// generated schema.
fn floor(tool: Tool) -> (u32, u32, u32) {
    match tool {
        Tool::Claude => (2, 1, 260),
        Tool::Codex => (0, 151, 0),
    }
}

/// File names to look for, best first.
///
/// npm installs on Windows write `claude.cmd` and `claude.ps1` and no `.exe`,
/// so looking only for an `.exe` reports a working install as missing. The
/// bare name on Windows is the shell script meant for Git Bash, which cannot
/// be started directly, so it is not looked for there.
fn candidate_names(tool: Tool) -> &'static [&'static str] {
    match (tool, cfg!(windows)) {
        (Tool::Claude, true) => &["claude.exe", "claude.cmd", "claude.bat"],
        (Tool::Codex, true) => &["codex.exe", "codex.cmd", "codex.bat"],
        (Tool::Claude, false) => &["claude"],
        (Tool::Codex, false) => &["codex"],
    }
}

/// Where installers put these tools when `PATH` has not caught up.
fn known_locations() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from) {
        out.push(home.join(".local").join("bin"));
        out.push(home.join(".claude").join("local"));
        out.push(home.join(".bun").join("bin"));
        out.push(home.join(".npm-global").join("bin"));
        if cfg!(windows) {
            out.push(home.join("AppData").join("Roaming").join("npm"));
            // Codex's own installer writes here and does not touch PATH.
            out.push(home.join("AppData").join("Local").join("Programs").join("OpenAI").join("Codex").join("bin"));
        }
    }
    if !cfg!(windows) {
        out.push(PathBuf::from("/usr/local/bin"));
        out.push(PathBuf::from("/opt/homebrew/bin"));
    }
    out
}

fn find_in(tool: Tool, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().flat_map(|dir| candidate_names(tool).iter().map(move |name| dir.join(name))).find(|path| path.is_file())
}

fn current_path() -> Vec<PathBuf> {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default()
}

/// Where a tool is, plus the wider `PATH` to give it when it was only found
/// on the login shell's.
pub(crate) struct Located {
    pub detected: Detected,
    pub path_env: Option<OsString>,
}

pub async fn detect(tool: Tool) -> Detected {
    locate(tool).await.detected
}

/// Both tools, checked at the same time.
pub async fn detect_all() -> Vec<Detected> {
    let (claude, codex) = tokio::join!(detect(Tool::Claude), detect(Tool::Codex));
    vec![claude, codex]
}

pub(crate) async fn locate(tool: Tool) -> Located {
    let mut dirs = current_path();
    dirs.extend(known_locations());
    let mut path_env = None;
    let mut program = find_in(tool, &dirs);
    if program.is_none() {
        let mut current = current_path();
        let known: HashSet<PathBuf> = current.iter().cloned().collect();
        let added: Vec<PathBuf> = login_shell_path().await.into_iter().filter(|dir| !known.contains(dir) && dir.is_dir()).collect();
        if let Some(found) = find_in(tool, &added) {
            program = Some(found);
            current.extend(added);
            path_env = std::env::join_paths(current).ok();
        }
    }

    let mut detected = Detected { tool, program: program.clone(), version: None, availability: Availability::NotFound, install_url: tool.install_url() };
    let Some(program) = program else {
        return Located { detected, path_env };
    };
    match version_of(&program).await {
        Some(raw) => {
            detected.availability = availability(tool, &raw);
            detected.version = Some(parse_version(&raw).map(|(a, b, c)| format!("{a}.{b}.{c}")).unwrap_or(raw));
        }
        None => detected.availability = Availability::Unresponsive(format!("{tool} is at {} but did not answer when asked its version", program.display())),
    }
    Located { detected, path_env }
}

/// Ready, or too old. A version string this crate cannot read counts as
/// ready: the format is the tool's to change, and refusing over it would
/// break installs that work.
fn availability(tool: Tool, raw: &str) -> Availability {
    let needs = floor(tool);
    match parse_version(raw) {
        Some(found) if found < needs => Availability::TooOld { found: format!("{}.{}.{}", found.0, found.1, found.2), needs: format!("{}.{}.{}", needs.0, needs.1, needs.2) },
        _ => Availability::Ready,
    }
}

async fn version_of(program: &Path) -> Option<String> {
    let mut cmd = probe_command(program);
    cmd.arg("--version");
    match tokio::time::timeout(VERSION_TIMEOUT, cmd.output()).await {
        Ok(Ok(out)) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            (!text.is_empty()).then_some(text)
        }
        _ => None,
    }
}

/// The first `major.minor.patch` in whatever the tool printed:
/// `2.1.287 (Claude Code)`, `codex-cli 0.159.1`.
pub(crate) fn parse_version(raw: &str) -> Option<(u32, u32, u32)> {
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
            i += 1;
        }
        let mut parts = raw[start..i].split('.').map(|p| p.parse::<u32>().ok());
        if let (Some(Some(a)), Some(Some(b)), Some(Some(c))) = (parts.next(), parts.next(), parts.next()) {
            return Some((a, b, c));
        }
    }
    None
}

/// The `PATH` a newly opened terminal would have, or nothing if the shell
/// would not say.
async fn login_shell_path() -> Vec<PathBuf> {
    // On Windows the durable PATH lives in the registry, and an installer
    // that edits it broadcasts a change this process never received.
    #[cfg(windows)]
    let mut cmd = {
        let mut cmd = probe_command("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", "[Environment]::GetEnvironmentVariable('PATH','Machine') + ';' + [Environment]::GetEnvironmentVariable('PATH','User')"]);
        cmd
    };
    // The user's own shell as a login shell, so the profile that adds a tool
    // to PATH is actually read.
    #[cfg(not(windows))]
    let mut cmd = {
        let mut cmd = probe_command(std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()));
        cmd.args(["-l", "-c", "printf %s \"$PATH\""]);
        cmd
    };
    match tokio::time::timeout(SHELL_TIMEOUT, cmd.output()).await {
        Ok(Ok(out)) if out.status.success() => std::env::split_paths(String::from_utf8_lossy(&out.stdout).trim()).filter(|p| !p.as_os_str().is_empty()).collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_each_tool_prints() {
        assert_eq!(parse_version("2.1.287 (Claude Code)"), Some((2, 1, 287)));
        assert_eq!(parse_version("codex-cli 0.159.1"), Some((0, 159, 1)));
        assert_eq!(parse_version("build 7 -- tool 1.2.3"), Some((1, 2, 3)), "a lone number is not a version");
        assert_eq!(parse_version("not installed"), None);
    }

    #[test]
    fn older_than_the_checked_protocol_is_too_old() {
        assert_eq!(availability(Tool::Claude, "2.1.287 (Claude Code)"), Availability::Ready);
        assert_eq!(availability(Tool::Claude, "2.1.9 (Claude Code)"), Availability::TooOld { found: "2.1.9".into(), needs: "2.1.260".into() }, "compared by number, not as text");
        assert_eq!(availability(Tool::Codex, "codex-cli 0.150.9"), Availability::TooOld { found: "0.150.9".into(), needs: "0.151.0".into() });
        assert_eq!(availability(Tool::Codex, "codex-cli 0.159.1"), Availability::Ready);
        assert_eq!(availability(Tool::Codex, "codex nightly"), Availability::Ready, "an unreadable version is not a reason to refuse");
    }

    #[test]
    fn windows_looks_for_the_npm_shim_and_never_the_bash_script() {
        for tool in Tool::ALL {
            let names = candidate_names(tool);
            if cfg!(windows) {
                assert!(names.iter().any(|n| n.ends_with(".cmd")), "{tool}: npm installs only a .cmd shim");
                assert!(names.iter().all(|n| n.contains('.')), "{tool}: the bare name is a shell script Windows cannot start");
            } else {
                assert_eq!(names.len(), 1);
            }
        }
    }

    #[tokio::test]
    async fn a_missing_tool_is_not_found_rather_than_an_error() {
        let found = find_in(Tool::Claude, &[std::env::temp_dir().join("agent-cli-no-such-folder")]);
        assert!(found.is_none());
    }
}
