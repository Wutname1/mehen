//! Gives a Mac app the PATH a terminal has.
//!
//! An app opened from the Dock or Finder is started by the system with a bare
//! PATH (/usr/bin:/bin:/usr/sbin:/sbin). npm, node, cargo, pnpm and anything
//! installed with Homebrew or a version manager live elsewhere, so every check
//! and update would report them as "not installed" even though they work fine
//! in a terminal. The login shell has the real PATH, so it is asked once at
//! start and the answer is adopted for this process and everything it runs.

use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MARK: &str = "__MEHEN_PATH__";

/// Directories worth having even when the shell could not be asked.
fn fallback_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/opt/homebrew/sbin"), PathBuf::from("/usr/local/bin")];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".cargo/bin"));
        dirs.push(home.join(".local/bin"));
    }
    dirs
}

/// What an interactive login shell prints for $PATH, or None if it would not
/// answer within a few seconds (a slow or broken rc file must not hold Mehen up).
fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into());
    // Markers around the value: rc files are free to print their own text.
    let script = format!("printf '%s%s%s' '{MARK}' \"$PATH\" '{MARK}'");
    let mut child = Command::new(shell).args(["-ilc", &script]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait().ok()? {
            Some(_) => break,
            None if started.elapsed() > Duration::from_secs(4) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(40)),
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let start = out.find(MARK)? + MARK.len();
    let end = out[start..].find(MARK)? + start;
    Some(out[start..end].to_string())
}

/// Puts the login shell's directories first, then what the app already had,
/// then the common install locations, without repeating any.
pub fn adopt_login_shell_path() {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut add = |dir: PathBuf| {
        if !dir.as_os_str().is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    };
    if let Some(shell_path) = login_shell_path() {
        std::env::split_paths(&OsString::from(shell_path)).for_each(&mut add);
    }
    if let Some(current) = std::env::var_os("PATH") {
        std::env::split_paths(&current).for_each(&mut add);
    }
    fallback_dirs().into_iter().for_each(&mut add);
    if let Ok(joined) = std::env::join_paths(dirs) {
        // Called first thing in `run`, before any thread exists to read the environment.
        unsafe { std::env::set_var("PATH", joined) };
    }
}
