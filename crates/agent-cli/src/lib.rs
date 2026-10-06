//! Drives the AI coding tools a person already has installed and signed in,
//! Claude Code (`claude`) and OpenAI Codex (`codex`), to do one task in one
//! folder, streaming what they do as [`Event`]s.
//!
//! No API keys and no stored credentials: each tool uses its own sign-in, and
//! this crate only asks it whether it can work. Nothing here waits for a
//! person either. Every permission prompt the tool raises is answered at once
//! from the task's [`Permissions`], and what is not allowed is also refused at
//! launch where the tool can be told so.
//!
//! Shared by Mehen (fixing code an update broke) and GitWyrm, so it depends on
//! neither and has no UI or framework types of its own.

mod cancel;
mod claude;
mod codex;
mod detect;
mod permissions;
mod process;

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use cancel::CancelToken;
pub use detect::{detect, detect_all};

/// Which tool to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Claude,
    Codex,
}

impl Tool {
    pub const ALL: [Tool; 2] = [Tool::Claude, Tool::Codex];

    /// The name its makers use.
    pub fn display_name(self) -> &'static str {
        match self {
            Tool::Claude => "Claude Code",
            Tool::Codex => "Codex",
        }
    }

    /// Where to send someone who does not have it yet.
    pub fn install_url(self) -> &'static str {
        match self {
            Tool::Claude => "https://docs.anthropic.com/en/docs/claude-code/setup",
            Tool::Codex => "https://developers.openai.com/codex/cli/",
        }
    }
}

impl fmt::Display for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// Whether a tool can be used on this computer right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "kebab-case")]
pub enum Availability {
    Ready,
    /// Installed, but older than the version whose protocol this crate was
    /// checked against. Updating the tool fixes it.
    TooOld { found: String, needs: String },
    /// On disk but did not answer when asked its version. Not reported as
    /// missing, so nobody is told to install a tool they already have.
    Unresponsive(String),
    NotFound,
}

#[derive(Debug, Clone, Serialize)]
pub struct Detected {
    pub tool: Tool,
    pub program: Option<PathBuf>,
    pub version: Option<String>,
    pub availability: Availability,
    pub install_url: &'static str,
}

/// What the agent may do without asking. Anything else it tries is refused
/// and reported as [`Event::Denied`].
///
/// Reading files is always allowed. Edits outside the task's folder are
/// always refused. Reaching the network goes with `run_commands`, because a
/// command can reach it anyway.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    pub edit_files: bool,
    pub run_commands: bool,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub tool: Tool,
    /// The folder the agent works in. Edits are confined to it.
    pub cwd: PathBuf,
    pub prompt: String,
    /// Added to the tool's own instructions rather than replacing them, so it
    /// keeps knowing how to use its tools.
    pub system_prompt: Option<String>,
    pub permissions: Permissions,
    /// Passed to the tool as written (`sonnet`, `gpt-5.5`, ...). `None` uses
    /// whatever the tool is set up for.
    pub model: Option<String>,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Event {
    /// A piece of the agent's reply.
    Text { text: String },
    /// A tool the agent is using. Sent as `running` when it starts and again
    /// with the same `id` as `done` or `failed`.
    ToolCall { id: String, title: String, status: String },
    /// A file the agent changed, relative to the task folder when inside it.
    /// Best effort: a shell command that writes files is not reported.
    FileChanged { path: String },
    /// Something the agent tried that [`Permissions`] did not allow.
    Denied { title: String, reason: String },
    /// Tokens used by the run so far. Each one replaces the last.
    Usage { input_tokens: Option<u64>, output_tokens: Option<u64> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// The agent reached the end of its turn. `false` when it stopped early
    /// on its own, for example at a length or step limit.
    pub finished: bool,
    /// The agent's final message.
    pub summary: String,
    /// The tool's own word for why it stopped (`end_turn`, `completed`,
    /// `error_max_turns`, ...).
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    NotInstalled(Tool),
    /// The tool could not be started, or is too old or unresponsive to use.
    Spawn(String),
    /// The tool said something this crate could not make sense of.
    Protocol(String),
    TimedOut,
    Cancelled,
    /// The tool started but could not do the task: not signed in, out of
    /// quota, or it stopped with an error. Carries the tool's own words.
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotInstalled(tool) => write!(f, "{tool} is not installed on this computer"),
            Error::Spawn(detail) | Error::Protocol(detail) | Error::Failed(detail) => f.write_str(detail),
            Error::TimedOut => f.write_str("The AI took too long and was stopped"),
            Error::Cancelled => f.write_str("Stopped"),
        }
    }
}

impl std::error::Error for Error {}

/// Runs one task to the end and returns the agent's final message.
///
/// Never waits for a person: permission prompts are answered from
/// `task.permissions`. Cancelling or timing out stops the tool and everything
/// it started.
pub async fn run(task: Task, cancel: CancelToken, on_event: impl Fn(Event) + Send + Sync) -> Result<Outcome, Error> {
    let detect::Located { detected: found, path_env } = detect::locate(task.tool).await;
    let program = match (&found.availability, found.program) {
        (Availability::Ready, Some(program)) => program,
        (Availability::NotFound, _) | (_, None) => return Err(Error::NotInstalled(task.tool)),
        (Availability::TooOld { found, needs }, _) => {
            return Err(Error::Spawn(format!("{} is version {found}, but {needs} or newer is needed. Updating it will fix this.", task.tool)));
        }
        (Availability::Unresponsive(detail), _) => return Err(Error::Spawn(detail.clone())),
    };
    if !task.cwd.is_dir() {
        return Err(Error::Spawn(format!("{} is not a folder", task.cwd.display())));
    }
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let launch = process::Launch { program, path_env };
    match task.tool {
        Tool::Claude => claude::run(&launch, &task, &cancel, &on_event).await,
        Tool::Codex => codex::run(&launch, &task, &cancel, &on_event).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serialize_with_a_kind_tag() {
        let event = Event::ToolCall { id: "t1".into(), title: "Edit src/lib.rs".into(), status: "running".into() };
        assert_eq!(serde_json::to_value(&event).unwrap(), serde_json::json!({ "kind": "tool-call", "id": "t1", "title": "Edit src/lib.rs", "status": "running" }));
        let usage = Event::Usage { input_tokens: Some(3), output_tokens: None };
        assert_eq!(serde_json::to_value(&usage).unwrap()["kind"], "usage");
        assert_eq!(serde_json::to_value(Event::FileChanged { path: "a".into() }).unwrap()["kind"], "file-changed");
    }

    #[test]
    fn tools_are_named_for_people_and_for_settings() {
        assert_eq!(serde_json::to_value(Tool::Claude).unwrap(), "claude");
        assert_eq!(serde_json::from_value::<Tool>(serde_json::json!("codex")).unwrap(), Tool::Codex);
        assert_eq!(Tool::Claude.to_string(), "Claude Code");
        assert_eq!(Tool::Codex.to_string(), "Codex");
    }

    #[test]
    fn availability_serializes_every_shape() {
        let too_old = Availability::TooOld { found: "2.0.1".into(), needs: "2.1.260".into() };
        assert_eq!(serde_json::to_value(&too_old).unwrap(), serde_json::json!({ "kind": "too-old", "detail": { "found": "2.0.1", "needs": "2.1.260" } }));
        assert_eq!(serde_json::to_value(Availability::Unresponsive("x".into())).unwrap()["detail"], "x");
        assert_eq!(serde_json::to_value(Availability::Ready).unwrap()["kind"], "ready");
    }
}
