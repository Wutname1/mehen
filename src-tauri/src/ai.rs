//! Fixing code after an update breaks it, with the user's own Claude Code or
//! Codex (whichever is installed and signed in). The engine decides what to
//! ask and checks the result; this hands the asking to the tool.

use std::path::PathBuf;
use std::time::Duration;

use agent_cli::{Availability, Permissions, Task, Tool};
use mehen_core::cancel::Cancel;
use mehen_core::fix::{self, FixEvent, FixOptions, FixOutcome};
use mehen_core::update::{self, UpdatePlan};
use mehen_core::Inventory;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;

/// One tool as the settings and update screens show it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTool {
    tool: Tool,
    name: String,
    version: Option<String>,
    ready: bool,
    /// Why it cannot be used yet.
    problem: Option<String>,
    install_url: String,
}

#[tauri::command]
pub async fn ai_tools() -> Vec<AiTool> {
    agent_cli::detect_all()
        .await
        .into_iter()
        .map(|d| AiTool {
            name: d.tool.to_string(),
            tool: d.tool,
            ready: matches!(d.availability, Availability::Ready),
            problem: match d.availability {
                Availability::Ready => None,
                Availability::NotFound => Some("Not installed".into()),
                Availability::TooOld { found, needs } => Some(format!("Version {found} is too old; {needs} or newer is needed")),
                Availability::Unresponsive(why) => Some(format!("Installed but not answering: {why}")),
            },
            version: d.version,
            install_url: d.install_url.to_string(),
        })
        .collect()
}

/// What the tool does, as the update screen shows it.
#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum Shown {
    Text { text: String },
    Tool { title: String, status: String },
    Denied { title: String, reason: String },
}

#[derive(Serialize)]
pub struct FixResult {
    outcome: FixOutcome,
    /// Checked again after a fix went in.
    inventory: Option<Inventory>,
}

/// Applies one repository's update and has `tool` fix the code until its
/// checks pass. Progress goes out as `mehen://fix` events.
#[tauri::command]
pub async fn fix_update(app: AppHandle, plans: Vec<UpdatePlan>, tool: Tool, build: bool, test: bool, commit: bool, push: bool) -> Result<FixResult, String> {
    let repo = plans.first().and_then(|p| p.repo.clone()).map(PathBuf::from).ok_or("Fixing code needs a git repository")?;
    let cancel = Cancel::default();
    *app.state::<AppState>().fix_cancel.lock().unwrap_or_else(|e| e.into_inner()) = cancel.clone();
    let options = FixOptions { tool: tool.to_string(), build, test, commit, push, rounds: 3 };
    let agent = |prompt: String, cancel: Cancel| {
        let (app, cwd) = (app.clone(), repo.clone());
        async move {
            let token = agent_cli::CancelToken::default();
            let watch = {
                let token = token.clone();
                tokio::spawn(async move {
                    cancel.cancelled().await;
                    token.cancel();
                })
            };
            let task = Task {
                tool,
                cwd,
                prompt,
                system_prompt: None,
                // Mehen runs the checks itself, so the tool only edits.
                permissions: Permissions { edit_files: true, run_commands: false },
                model: None,
                timeout: Some(Duration::from_secs(20 * 60)),
            };
            let result = agent_cli::run(task, token, move |event| {
                let shown = match event {
                    agent_cli::Event::Text { text } => Some(Shown::Text { text }),
                    agent_cli::Event::ToolCall { title, status, .. } => Some(Shown::Tool { title, status }),
                    agent_cli::Event::Denied { title, reason } => Some(Shown::Denied { title, reason }),
                    _ => None,
                };
                if let Some(shown) = shown {
                    let _ = app.emit("mehen://fix", shown);
                }
            })
            .await;
            watch.abort();
            result.map(|o| o.summary).map_err(|e| e.to_string())
        }
    };
    let outcome = fix::fix(plans, options, &cancel, |step, cancel| async move { update::run_step(&step, &cancel).await }, agent, |e: FixEvent| {
        let _ = app.emit("mehen://fix", e);
    })
    .await;
    let inventory = if outcome.ok { Some(crate::check_now(&app, false, None).await?) } else { None };
    Ok(FixResult { outcome, inventory })
}

/// Stops the fix running now; its files are put back.
#[tauri::command]
pub fn cancel_fix(app: AppHandle) {
    app.state::<AppState>().fix_cancel.lock().unwrap_or_else(|e| e.into_inner()).cancel();
}
