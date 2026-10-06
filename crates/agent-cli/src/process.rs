//! Starting a tool, talking to it a line at a time, and stopping it together
//! with everything it started.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};

use crate::{CancelToken, Error};

/// Windows: start without a console window, which would otherwise flash up
/// over a GUI app for every run.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How long a tool gets to exit on its own after its input is closed.
const EXIT_GRACE: Duration = Duration::from_secs(3);

/// Lines of stderr kept for an error message.
const STDERR_LINES: usize = 8;

/// The program to start, as discovery found it.
pub(crate) struct Launch {
    pub program: PathBuf,
    /// Replaces the child's `PATH` when discovery had to widen it (see
    /// `detect`), so the tool's own commands find what it was found with.
    pub path_env: Option<OsString>,
}

/// A running tool with its pipes taken.
pub(crate) struct Running {
    pub child: Child,
    pub stdin: ChildStdin,
    pub lines: Lines<BufReader<ChildStdout>>,
    pub stderr: StderrTail,
}

/// Starts one of the tools with all three pipes open.
pub(crate) fn spawn(launch: &Launch, args: &[String], cwd: &Path) -> Result<Running, Error> {
    let mut cmd = Command::new(&launch.program);
    cmd.args(args).current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    if let Some(path) = &launch.path_env {
        cmd.env("PATH", path);
    }
    scrub_bundled_env(&mut cmd);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    // Its own process group, so stopping it reaches the shells it starts.
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd.spawn().map_err(|e| Error::Spawn(format!("Could not start {}: {e}", launch.program.display())))?;
    let (Some(stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        return Err(Error::Spawn(format!("{} started without its input and output", launch.program.display())));
    };
    Ok(Running { child, stdin, lines: BufReader::new(stdout).lines(), stderr: StderrTail::collect(stderr) })
}

/// A command for a quick probe (`--version`, the login shell's `PATH`).
pub(crate) fn probe_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    scrub_bundled_env(&mut cmd);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Undoes the library paths an AppImage puts on its own process, which would
/// otherwise reach a system binary and break its loader. The runtime saves
/// each original as `<VAR>_ORIG`.
#[cfg(target_os = "linux")]
fn scrub_bundled_env(cmd: &mut Command) {
    const OVERRIDDEN: &[&str] = &["LD_LIBRARY_PATH", "LD_PRELOAD", "GIO_MODULE_DIR", "GSETTINGS_SCHEMA_DIR", "GDK_PIXBUF_MODULE_FILE", "QT_PLUGIN_PATH", "PYTHONPATH", "PERLLIB", "XDG_DATA_DIRS"];
    if std::env::var_os("APPIMAGE").is_none() && std::env::var_os("APPDIR").is_none() {
        return;
    }
    for var in OVERRIDDEN {
        match std::env::var_os(format!("{var}_ORIG")) {
            Some(original) => cmd.env(var, original),
            None => cmd.env_remove(var),
        };
    }
}

#[cfg(not(target_os = "linux"))]
fn scrub_bundled_env(_cmd: &mut Command) {}

/// Stops a process and every process it started: an agent runs commands
/// through a shell, and stopping only the agent leaves them running.
pub(crate) async fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("taskkill");
        cmd.args(["/T", "/F", "/PID", &pid.to_string()]).creation_flags(CREATE_NO_WINDOW);
        let _ = cmd.output().await;
    }
    #[cfg(unix)]
    {
        let _ = Command::new("kill").args(["-KILL", &format!("-{pid}")]).output().await;
    }
}

pub(crate) enum Stopped<T> {
    Done(T),
    Cancelled,
    TimedOut,
}

/// Runs `work` until it finishes, the run is cancelled, or time runs out.
pub(crate) async fn until_stopped<T>(work: impl Future<Output = T>, cancel: &CancelToken, timeout: Option<Duration>) -> Stopped<T> {
    let deadline = async {
        match timeout {
            Some(limit) => tokio::time::sleep(limit).await,
            None => std::future::pending().await,
        }
    };
    tokio::select! {
        done = work => Stopped::Done(done),
        _ = cancel.cancelled() => Stopped::Cancelled,
        _ = deadline => Stopped::TimedOut,
    }
}

/// Closes the tool's input so it can exit cleanly, then stops whatever is
/// still running after a short wait.
pub(crate) async fn finish(child: &mut Child, stdin: &mut ChildStdin) {
    let _ = stdin.shutdown().await;
    if tokio::time::timeout(EXIT_GRACE, child.wait()).await.is_err() {
        stop(child).await;
    }
}

/// Stops the tool and its children now.
pub(crate) async fn stop(child: &mut Child) {
    if let Some(pid) = child.id() {
        kill_tree(pid).await;
    }
    let _ = child.start_kill();
    let _ = tokio::time::timeout(EXIT_GRACE, child.wait()).await;
}

pub(crate) async fn write_line(stdin: &mut ChildStdin, value: &Value) -> std::io::Result<()> {
    let mut line = value.to_string();
    line.push('\n');
    stdin.write_all(line.as_bytes()).await?;
    stdin.flush().await
}

/// The last few lines a tool wrote to stderr. When it exits before answering
/// (an expired sign-in, a flag it will not accept) that is the only account
/// of why.
#[derive(Clone, Default)]
pub(crate) struct StderrTail(Arc<Mutex<VecDeque<String>>>);

impl StderrTail {
    pub(crate) fn collect(stderr: ChildStderr) -> Self {
        let tail = StderrTail::default();
        let lines = tail.0.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                let mut lines = lines.lock().unwrap_or_else(|e| e.into_inner());
                if lines.len() >= STDERR_LINES {
                    lines.pop_front();
                }
                lines.push_back(line);
            }
        });
        tail
    }

    pub(crate) fn text(&self) -> String {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect::<Vec<_>>().join("\n")
    }

    /// "`what`", followed by what the tool said, if anything.
    pub(crate) fn explain(&self, what: &str) -> String {
        let said = self.text();
        if said.trim().is_empty() { what.to_string() } else { format!("{what}: {said}") }
    }
}
