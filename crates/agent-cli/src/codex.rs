//! Driving Codex through its own app-server.
//!
//! Codex has no stream mode like Claude Code's; `codex app-server` speaks
//! JSON-RPC on stdio, one message per line. Shapes taken from real 0.151.0
//! sessions and the schema the binary generates (`codex app-server
//! generate-json-schema`), rechecked against 0.159.1:
//!
//! - `initialize` (then the `initialized` notification), `thread/start`
//!   answering `result.thread.id`, and `turn/start` answering `result.turn`
//!   while the turn runs. The finished turn arrives later as `turn/completed`.
//! - Prose arrives as `item/agentMessage/delta`; tools as `item/started` and
//!   `item/completed`; spend as `thread/tokenUsage/updated`.
//! - Approvals are requests FROM Codex (`item/commandExecution/
//!   requestApproval`, `item/fileChange/requestApproval`, `item/permissions/
//!   requestApproval`, and the older `execCommandApproval` /
//!   `applyPatchApproval`). Each must be answered or the turn hangs, and each
//!   family has its own words: `accept`/`decline` for the item requests,
//!   `approved`/`denied` for the older two. A wrong word reads as a refusal.
//! - A file-change approval names no files. They arrive earlier, on
//!   `item/started` for the same item id.
//!
//! The bound is Codex's sandbox, set per thread and per turn: read-only, or
//! writes inside the folder only, with the network off unless commands are
//! allowed. Approval prompts are answered from [`Permissions`] on top of it.
//!
//! Closing stdin kills the server mid-request, so it stays open until the
//! turn is over.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::permissions::{self, Kind, Verdict};
use crate::process::{self, Launch, Running};
use crate::{CancelToken, Error, Event, Outcome, Permissions, Task};

const INTERRUPT_GRACE: Duration = Duration::from_secs(2);

/// Thread settings that make Codex's own enforcement match [`Permissions`].
///
/// `approvalPolicy`: with commands allowed, `never`, so Codex runs them in
/// the sandbox and never stops to ask for more. Without, `untrusted`, so
/// anything beyond its built-in list of harmless read commands comes here to
/// be declined. `ephemeral` keeps these runs out of the person's history.
fn thread_params(task: &Task) -> Value {
    let p = task.permissions;
    let mut params = json!({
        "cwd": task.cwd.to_string_lossy(),
        "sandbox": if p.edit_files { "workspace-write" } else { "read-only" },
        "approvalPolicy": if p.run_commands { "never" } else { "untrusted" },
        "ephemeral": true,
    });
    // Tools that act outside the folder without going through the sandbox or
    // an approval: the person's connected apps, a browser, the desktop. Web
    // search runs on OpenAI's side, so the sandbox's network switch does not
    // reach it either.
    let mut config = json!({ "features": { "apps": false, "browser_use": false, "browser_use_external": false, "in_app_browser": false, "computer_use": false, "image_generation": false } });
    if !p.run_commands {
        config["web_search"] = json!("disabled");
    }
    params["config"] = config;
    if let Some(prompt) = task.system_prompt.as_deref().filter(|s| !s.trim().is_empty()) {
        params["developerInstructions"] = json!(prompt);
    }
    if let Some(model) = task.model.as_deref().filter(|m| !m.trim().is_empty()) {
        params["model"] = json!(model);
    }
    params
}

/// The sandbox again, per turn, because only the turn-level policy carries
/// the network switch.
fn turn_params(thread_id: &str, task: &Task) -> Value {
    let p = task.permissions;
    let sandbox = if p.edit_files { json!({ "type": "workspaceWrite", "networkAccess": p.run_commands }) } else { json!({ "type": "readOnly", "networkAccess": p.run_commands }) };
    json!({ "threadId": thread_id, "input": [{ "type": "text", "text": task.prompt }], "sandboxPolicy": sandbox })
}

/// Tool-like items worth showing; the agent's own messages, reasoning and
/// bookkeeping are not.
const TOOL_ITEMS: &[&str] = &["commandExecution", "fileChange", "mcpToolCall", "dynamicToolCall", "webSearch", "imageView", "collabAgentToolCall", "imageGeneration"];

#[derive(Default)]
struct Step {
    events: Vec<Event>,
    reply: Option<Value>,
}

struct Session {
    cwd: PathBuf,
    permissions: Permissions,
    /// Files per file-change item, from `item/started`.
    file_paths: HashMap<String, Vec<String>>,
    changed: HashSet<String>,
    responses: HashMap<i64, Result<Value, Error>>,
    turn: Option<Value>,
    last_message: String,
    last_error: Option<String>,
}

impl Session {
    fn new(task: &Task) -> Self {
        Session { cwd: task.cwd.clone(), permissions: task.permissions, file_paths: HashMap::new(), changed: HashSet::new(), responses: HashMap::new(), turn: None, last_message: String::new(), last_error: None }
    }

    fn handle(&mut self, msg: &Value) -> Step {
        let mut step = Step::default();
        let params = msg.get("params").unwrap_or(&Value::Null);
        match (msg.get("id"), msg.get("method").and_then(Value::as_str)) {
            (Some(id), None) => {
                let Some(id) = id.as_i64() else { return step };
                let result = match msg.get("error") {
                    Some(error) => Err(Error::Failed(error.get("message").and_then(Value::as_str).unwrap_or("Codex reported a problem").to_string())),
                    None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                };
                self.responses.insert(id, result);
            }
            (Some(id), Some(method)) if is_approval(method) => step.reply = Some(json!({ "jsonrpc": "2.0", "id": id, "result": self.answer(method, params, &mut step.events) })),
            // Any other request from Codex (a question for the user, a tool
            // only a UI provides) would hold the turn forever unanswered, so
            // it is refused out loud.
            (Some(id), Some(_)) => step.reply = Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "This host does not handle this request" } })),
            (None, Some(method)) => self.notification(method, params, &mut step.events),
            (None, None) => {}
        }
        step
    }

    /// The `result` for one approval request, in that request's own words.
    fn answer(&mut self, method: &str, params: &Value, events: &mut Vec<Event>) -> Value {
        if method == "item/permissions/requestApproval" {
            // The sandbox already grants exactly what Permissions allow, so a
            // request for more is a request for something not allowed.
            let reason = params.get("reason").and_then(Value::as_str).filter(|r| !r.is_empty());
            events.push(Event::Denied { title: reason.map(|r| format!("Extra access: {r}")).unwrap_or_else(|| "Extra access".into()), reason: "Access beyond this task's permissions is not allowed".into() });
            return json!({ "permissions": {} });
        }
        let legacy = matches!(method, "execCommandApproval" | "applyPatchApproval");
        let (title, verdict) = if method.contains("ommand") {
            let command = match params.get("command") {
                Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
                Some(Value::String(text)) => text.clone(),
                _ => "a command".into(),
            };
            let kind = if params.get("networkApprovalContext").is_some_and(|n| !n.is_null()) { Kind::Network } else { Kind::Command };
            (format!("Run {}", shorten(unwrap_shell(&command))), permissions::judge(kind, "command", &[], &self.cwd, self.permissions))
        } else {
            let mut paths: Vec<String> = match params.get("fileChanges").and_then(Value::as_object) {
                Some(changes) => changes.keys().cloned().collect(),
                None => params.get("itemId").and_then(Value::as_str).and_then(|item| self.file_paths.get(item)).cloned().unwrap_or_default(),
            };
            if let Some(root) = params.get("grantRoot").and_then(Value::as_str) {
                paths.push(root.to_string());
            }
            (edit_title(&self.cwd, &paths), permissions::judge(Kind::Edit, "edit", &paths, &self.cwd, self.permissions))
        };
        if let Verdict::Deny(reason) = &verdict {
            events.push(Event::Denied { title, reason: reason.clone() });
        }
        let word = match (verdict.allowed(), legacy) {
            (true, false) => "accept",
            (false, false) => "decline",
            (true, true) => "approved",
            (false, true) => "denied",
        };
        json!({ "decision": word })
    }

    fn notification(&mut self, method: &str, params: &Value, events: &mut Vec<Event>) {
        match method {
            "item/agentMessage/delta" => {
                if let Some(delta) = params.get("delta").and_then(Value::as_str).filter(|d| !d.is_empty()) {
                    events.push(Event::Text { text: delta.to_string() });
                }
            }
            "item/started" | "item/completed" => {
                let Some(item) = params.get("item") else { return };
                let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
                let id = item.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                if kind == "agentMessage" && method == "item/completed"
                    && let Some(text) = item.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
                        self.last_message = text.to_string();
                    }
                if !TOOL_ITEMS.contains(&kind) {
                    return;
                }
                let paths: Vec<String> = item.get("changes").and_then(Value::as_array).into_iter().flatten().filter_map(|c| c.get("path").and_then(Value::as_str)).map(str::to_string).collect();
                if kind == "fileChange" {
                    self.file_paths.insert(id.clone(), paths.clone());
                }
                let status = match (method, item.get("status").and_then(Value::as_str)) {
                    ("item/started", _) => "running",
                    (_, Some("failed" | "declined")) => "failed",
                    _ => "done",
                };
                events.push(Event::ToolCall { id, title: item_title(&self.cwd, kind, item, &paths), status: status.into() });
                if kind == "fileChange" && method == "item/completed" && status == "done" {
                    for path in paths.iter().map(|p| permissions::display_path(&self.cwd, p)) {
                        if self.changed.insert(path.clone()) {
                            events.push(Event::FileChanged { path });
                        }
                    }
                }
            }
            "thread/tokenUsage/updated" => {
                // `total` is the whole thread, which is the whole run.
                if let Some(total) = params.pointer("/tokenUsage/total") {
                    events.push(Event::Usage { input_tokens: total.get("inputTokens").and_then(Value::as_u64), output_tokens: total.get("outputTokens").and_then(Value::as_u64) });
                }
            }
            "error" => {
                if !params.get("willRetry").and_then(Value::as_bool).unwrap_or(false) {
                    self.last_error = params.pointer("/error/message").and_then(Value::as_str).map(str::to_string);
                }
            }
            "turn/completed" => self.turn = Some(params.get("turn").cloned().unwrap_or(Value::Null)),
            _ => {}
        }
    }

    fn outcome(&self, turn: &Value) -> Result<Outcome, Error> {
        let status = turn.get("status").and_then(Value::as_str);
        match status {
            Some("completed") => Ok(Outcome { finished: true, summary: self.last_message.clone(), stop_reason: Some("completed".into()) }),
            Some("failed") => {
                let message = turn.pointer("/error/message").and_then(Value::as_str).map(str::to_string).or_else(|| self.last_error.clone());
                Err(Error::Failed(message.unwrap_or_else(|| "Codex could not finish this task".into())))
            }
            // Interrupted without us asking, or a status this build has not
            // seen: not a clean finish either way.
            other => Ok(Outcome { finished: false, summary: self.last_message.clone(), stop_reason: other.map(str::to_string) }),
        }
    }
}

fn is_approval(method: &str) -> bool {
    method.ends_with("requestApproval") || matches!(method, "execCommandApproval" | "applyPatchApproval")
}

/// The command a person would recognise. Codex wraps each one in the shell
/// it runs it with, e.g. `"C:\...\pwsh.exe" -Command 'mkdir x'` (measured on
/// 0.159.1) or `bash -lc 'mkdir x'`.
fn unwrap_shell(command: &str) -> &str {
    let command = command.trim();
    [" -Command '", " -lc '", " -c '"]
        .iter()
        .find_map(|flag| command.find(flag).map(|at| &command[at + flag.len()..]))
        .and_then(|inner| inner.strip_suffix('\''))
        .unwrap_or(command)
}

fn shorten(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() > 120 { format!("{}...", text.chars().take(117).collect::<String>()) } else { text.to_string() }
}

fn edit_title(cwd: &Path, paths: &[String]) -> String {
    let shown: Vec<String> = paths.iter().map(|p| permissions::display_path(cwd, p)).collect();
    match shown.as_slice() {
        [] => "Edit files".into(),
        [only] => format!("Edit {only}"),
        many => format!("Edit {} files: {}", many.len(), shorten(&many.join(", "))),
    }
}

fn item_title(cwd: &Path, kind: &str, item: &Value, paths: &[String]) -> String {
    let field = |name: &str| item.get(name).and_then(Value::as_str);
    match kind {
        "commandExecution" => format!("Run {}", shorten(unwrap_shell(field("command").unwrap_or("a command")))),
        "fileChange" => edit_title(cwd, paths),
        "mcpToolCall" => format!("{}: {}", field("server").unwrap_or("tool"), field("tool").unwrap_or("call")),
        "webSearch" => field("query").filter(|q| !q.trim().is_empty()).map(|q| format!("Search the web for {}", shorten(q))).unwrap_or_else(|| "Search the web".into()),
        "imageView" => format!("View {}", field("path").map(|p| permissions::display_path(cwd, p)).unwrap_or_else(|| "an image".into())),
        other => field("tool").unwrap_or(other).to_string(),
    }
}

/// One Codex app-server conversation, read and answered a line at a time.
struct Driver<'a, F> {
    running: &'a mut Running,
    session: Session,
    on_event: &'a F,
    next_id: i64,
}

impl<F: Fn(Event) + Send + Sync> Driver<'_, F> {
    async fn send(&mut self, message: Value) -> Result<(), Error> {
        process::write_line(&mut self.running.stdin, &message).await.map_err(|e| Error::Failed(self.running.stderr.explain(&format!("Codex stopped listening: {e}"))))
    }

    /// Reads and handles one line. `false` once Codex has closed its output.
    async fn pump(&mut self) -> Result<bool, Error> {
        let Ok(Some(line)) = self.running.lines.next_line().await else { return Ok(false) };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { return Ok(true) };
        let step = self.session.handle(&msg);
        for event in step.events {
            (self.on_event)(event);
        }
        if let Some(reply) = step.reply {
            self.send(reply).await?;
        }
        Ok(true)
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, Error> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await?;
        loop {
            if let Some(result) = self.session.responses.remove(&id) {
                return result;
            }
            if !self.pump().await? {
                return Err(Error::Failed(self.running.stderr.explain("Codex stopped unexpectedly")));
            }
        }
    }

    async fn drive(&mut self, task: &Task, turn_id: &mut Option<(String, String)>) -> Result<Outcome, Error> {
        self.request("initialize", json!({ "clientInfo": { "name": "agent-cli", "version": env!("CARGO_PKG_VERSION") } })).await?;
        self.send(json!({ "jsonrpc": "2.0", "method": "initialized" })).await?;
        let started = self.request("thread/start", thread_params(task)).await?;
        let thread = started.pointer("/thread/id").and_then(Value::as_str).ok_or_else(|| Error::Protocol("Codex started a conversation without an id".into()))?.to_string();
        let turn = self.request("turn/start", turn_params(&thread, task)).await?;
        if let Some(id) = turn.pointer("/turn/id").and_then(Value::as_str) {
            *turn_id = Some((thread, id.to_string()));
        }
        loop {
            if let Some(turn) = self.session.turn.take() {
                return self.session.outcome(&turn);
            }
            if !self.pump().await? {
                let said = self.session.last_error.clone().unwrap_or_else(|| self.running.stderr.explain("Codex stopped unexpectedly"));
                return Err(Error::Failed(said));
            }
        }
    }
}

pub(crate) async fn run(launch: &Launch, task: &Task, cancel: &CancelToken, on_event: &(impl Fn(Event) + Send + Sync)) -> Result<Outcome, Error> {
    let mut running = process::spawn(launch, &["app-server".to_string()], &task.cwd)?;
    let mut turn_id = None;
    let mut driver = Driver { running: &mut running, session: Session::new(task), on_event, next_id: 1 };
    let stopped = process::until_stopped(driver.drive(task, &mut turn_id), cancel, task.timeout).await;
    match stopped {
        process::Stopped::Done(result) => {
            process::finish(&mut driver.running.child, &mut driver.running.stdin).await;
            result
        }
        process::Stopped::Cancelled => {
            // `turn/interrupt` is a request and needs the turn's id, unlike
            // the bare notification an earlier client sent and Codex ignored.
            if let Some((thread, turn)) = turn_id {
                let interrupt = json!({ "jsonrpc": "2.0", "id": driver.next_id, "method": "turn/interrupt", "params": { "threadId": thread, "turnId": turn } });
                if driver.send(interrupt).await.is_ok() {
                    let _ = tokio::time::timeout(INTERRUPT_GRACE, async {
                        while driver.session.turn.is_none() && driver.pump().await.unwrap_or(false) {}
                    })
                    .await;
                }
            }
            process::stop(&mut driver.running.child).await;
            Err(Error::Cancelled)
        }
        process::Stopped::TimedOut => {
            process::stop(&mut driver.running.child).await;
            Err(Error::TimedOut)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(permissions: Permissions) -> Task {
        Task { tool: crate::Tool::Codex, cwd: std::env::temp_dir().join("agent-cli-codex-test"), prompt: "fix it".into(), system_prompt: Some("Be brief.".into()), permissions, model: None, timeout: None }
    }

    fn all() -> Permissions {
        Permissions { edit_files: true, run_commands: true }
    }

    #[test]
    fn the_sandbox_matches_the_permissions() {
        let none = thread_params(&task(Permissions::default()));
        assert_eq!(none["sandbox"], "read-only");
        assert_eq!(none["approvalPolicy"], "untrusted", "commands beyond harmless reads must come here to be declined");
        assert_eq!(none["developerInstructions"], "Be brief.");
        let both = thread_params(&task(all()));
        assert_eq!(both["sandbox"], "workspace-write");
        assert_eq!(both["approvalPolicy"], "never", "a run that may run commands must never stop to ask");
        let turn = turn_params("t1", &task(Permissions { edit_files: true, run_commands: false }));
        assert_eq!(turn["sandboxPolicy"], json!({ "type": "workspaceWrite", "networkAccess": false }));
        assert_eq!(turn["input"][0], json!({ "type": "text", "text": "fix it" }));
        assert_eq!(turn_params("t1", &task(Permissions::default()))["sandboxPolicy"]["type"], "readOnly");
    }

    #[test]
    fn approvals_use_each_requests_own_words() {
        let mut refused = Session::new(&task(Permissions::default()));
        let mut allowed = Session::new(&task(all()));
        let command = |method: &str, id: i64| json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": { "itemId": "c1", "command": "cargo build" } });
        for (method, yes, no) in [("item/commandExecution/requestApproval", "accept", "decline"), ("execCommandApproval", "approved", "denied")] {
            let reply = allowed.handle(&command(method, 7)).reply.unwrap();
            assert_eq!(reply["id"], 7);
            assert_eq!(reply["result"]["decision"], yes, "{method}");
            let step = refused.handle(&command(method, 8));
            assert_eq!(step.reply.unwrap()["result"]["decision"], no, "{method}");
            assert_eq!(step.events, vec![Event::Denied { title: "Run cargo build".into(), reason: permissions::NO_COMMANDS.into() }]);
        }
    }

    /// Recorded from 0.159.1 on Windows, and the shape Codex uses elsewhere.
    #[test]
    fn commands_are_shown_without_the_shell_around_them() {
        assert_eq!(unwrap_shell(r#""C:\\Program Files\\PowerShell\\7\\pwsh.exe" -Command 'mkdir made-by-shell'"#), "mkdir made-by-shell");
        assert_eq!(unwrap_shell("/bin/bash -lc 'cargo build'"), "cargo build");
        assert_eq!(unwrap_shell("npm test"), "npm test");
        let step = Session::new(&task(all())).handle(&json!({ "method": "item/started", "params": { "item": { "type": "webSearch", "id": "w", "query": "" } } }));
        assert_eq!(step.events[0], Event::ToolCall { id: "w".into(), title: "Search the web".into(), status: "running".into() });
    }

    #[test]
    fn tools_that_act_outside_the_sandbox_are_switched_off() {
        let none = thread_params(&task(Permissions::default()));
        assert_eq!(none["config"]["web_search"], "disabled", "web search runs on OpenAI's side, past the sandbox's network switch");
        for feature in ["apps", "browser_use", "computer_use"] {
            assert_eq!(none["config"]["features"][feature], false, "{feature}");
        }
        assert!(thread_params(&task(all()))["config"].get("web_search").is_none());
    }

    #[test]
    fn a_legacy_command_names_its_words_joined() {
        let step = Session::new(&task(Permissions::default())).handle(&json!({ "id": 3, "method": "execCommandApproval", "params": { "command": ["git", "push"] } }));
        assert_eq!(step.events, vec![Event::Denied { title: "Run git push".into(), reason: permissions::NO_COMMANDS.into() }]);
    }

    #[test]
    fn a_network_request_is_judged_as_the_network() {
        let t = task(Permissions { edit_files: true, run_commands: false });
        let step = Session::new(&t).handle(&json!({ "id": 4, "method": "item/commandExecution/requestApproval", "params": { "command": "curl x", "networkApprovalContext": { "host": "x", "protocol": "https" } } }));
        assert_eq!(step.events[0], Event::Denied { title: "Run curl x".into(), reason: permissions::NO_NETWORK.into() });
    }

    /// The approval names no files; they come from the earlier `item/started`
    /// for the same item, recorded from a 0.151.0 session.
    #[test]
    fn file_change_approvals_are_judged_on_the_files_from_item_started() {
        let t = task(Permissions { edit_files: true, run_commands: false });
        let mut session = Session::new(&t);
        let inside = t.cwd.join("src").join("a.rs").to_string_lossy().into_owned();
        let started = session.handle(&json!({ "method": "item/started", "params": { "item": { "type": "fileChange", "id": "fc-1", "status": "inProgress", "changes": [{ "path": inside, "kind": { "type": "update" }, "diff": "" }] } } }));
        assert_eq!(started.events, vec![Event::ToolCall { id: "fc-1".into(), title: "Edit src/a.rs".into(), status: "running".into() }]);
        let reply = session.handle(&json!({ "id": 9, "method": "item/fileChange/requestApproval", "params": { "itemId": "fc-1", "threadId": "t", "turnId": "u" } })).reply.unwrap();
        assert_eq!(reply["result"]["decision"], "accept");

        let outside = std::env::temp_dir().join("b.rs").to_string_lossy().into_owned();
        session.handle(&json!({ "method": "item/started", "params": { "item": { "type": "fileChange", "id": "fc-2", "changes": [{ "path": inside }, { "path": outside }] } } }));
        let step = session.handle(&json!({ "id": 10, "method": "item/fileChange/requestApproval", "params": { "itemId": "fc-2" } }));
        assert_eq!(step.reply.unwrap()["result"]["decision"], "decline", "one file outside the folder refuses the whole change");
        assert_eq!(step.events.len(), 1);

        let unknown = session.handle(&json!({ "id": 11, "method": "item/fileChange/requestApproval", "params": { "itemId": "never-started" } }));
        assert_eq!(unknown.reply.unwrap()["result"]["decision"], "decline", "a change naming no files is not assumed to stay inside");

        let grant = session.handle(&json!({ "id": 12, "method": "item/fileChange/requestApproval", "params": { "itemId": "fc-1", "grantRoot": std::env::temp_dir() } }));
        assert_eq!(grant.reply.unwrap()["result"]["decision"], "decline", "asking to write outside the folder for the rest of the session");
    }

    #[test]
    fn a_request_for_extra_access_is_always_refused() {
        let step = Session::new(&task(all())).handle(&json!({ "id": 5, "method": "item/permissions/requestApproval", "params": { "reason": "write to the workspace", "permissions": { "network": { "enabled": true } } } }));
        assert_eq!(step.reply.unwrap()["result"], json!({ "permissions": {} }));
        assert!(matches!(&step.events[0], Event::Denied { title, .. } if title.contains("write to the workspace")));
    }

    #[test]
    fn other_requests_from_codex_are_refused_out_loud() {
        let step = Session::new(&task(all())).handle(&json!({ "id": 6, "method": "item/tool/requestUserInput", "params": {} }));
        let reply = step.reply.unwrap();
        assert_eq!(reply["id"], 6);
        assert_eq!(reply["error"]["code"], -32601);
    }

    /// Shapes from a real 0.151.0 turn.
    #[test]
    fn notifications_become_events() {
        let t = task(all());
        let mut session = Session::new(&t);
        let text = session.handle(&json!({ "method": "item/agentMessage/delta", "params": { "delta": "PONG", "threadId": "t", "itemId": "m" } }));
        assert_eq!(text.events, vec![Event::Text { text: "PONG".into() }]);
        assert!(session.handle(&json!({ "method": "item/started", "params": { "item": { "type": "agentMessage", "id": "m" } } })).events.is_empty(), "prose is already carried by the deltas");
        let run = session.handle(&json!({ "method": "item/started", "params": { "item": { "type": "commandExecution", "id": "c", "command": "npm test", "status": "inProgress" } } }));
        assert_eq!(run.events, vec![Event::ToolCall { id: "c".into(), title: "Run npm test".into(), status: "running".into() }]);
        let failed = session.handle(&json!({ "method": "item/completed", "params": { "item": { "type": "commandExecution", "id": "c", "command": "npm test", "status": "failed", "exitCode": 1 } } }));
        assert_eq!(failed.events[0], Event::ToolCall { id: "c".into(), title: "Run npm test".into(), status: "failed".into() });
        let path = t.cwd.join("a.rs").to_string_lossy().into_owned();
        let edited = session.handle(&json!({ "method": "item/completed", "params": { "item": { "type": "fileChange", "id": "f", "status": "completed", "changes": [{ "path": path, "kind": { "type": "update" }, "diff": "" }] } } }));
        assert_eq!(edited.events, vec![Event::ToolCall { id: "f".into(), title: "Edit a.rs".into(), status: "done".into() }, Event::FileChanged { path: "a.rs".into() }]);
        let usage = session.handle(&json!({ "method": "thread/tokenUsage/updated", "params": { "tokenUsage": { "total": { "totalTokens": 21892, "inputTokens": 21884, "outputTokens": 8 }, "last": { "totalTokens": 21892 }, "modelContextWindow": 258400 } } }));
        assert_eq!(usage.events, vec![Event::Usage { input_tokens: Some(21884), output_tokens: Some(8) }]);
    }

    #[test]
    fn the_turn_ends_with_the_last_message() {
        let mut session = Session::new(&task(all()));
        session.handle(&json!({ "method": "item/completed", "params": { "item": { "type": "agentMessage", "id": "m", "text": "Fixed the import." } } }));
        session.handle(&json!({ "method": "turn/completed", "params": { "threadId": "t", "turn": { "id": "u", "items": [], "status": "completed" } } }));
        let turn = session.turn.take().unwrap();
        assert_eq!(session.outcome(&turn).unwrap(), Outcome { finished: true, summary: "Fixed the import.".into(), stop_reason: Some("completed".into()) });
        let failed = session.outcome(&json!({ "status": "failed", "error": { "message": "You've hit your usage limit." } }));
        assert_eq!(failed, Err(Error::Failed("You've hit your usage limit.".into())));
        for odd in [json!({ "status": "interrupted" }), json!({ "status": "something_new" }), Value::Null] {
            assert!(!session.outcome(&odd).unwrap().finished, "{odd} is not a clean finish");
        }
    }

    #[test]
    fn replies_to_our_requests_are_kept_by_id() {
        let mut session = Session::new(&task(all()));
        session.handle(&json!({ "jsonrpc": "2.0", "id": 2, "result": { "thread": { "id": "th" } } }));
        session.handle(&json!({ "jsonrpc": "2.0", "id": 3, "error": { "code": 1, "message": "Not signed in" } }));
        assert_eq!(session.responses.remove(&2).unwrap().unwrap()["thread"]["id"], "th");
        assert_eq!(session.responses.remove(&3).unwrap(), Err(Error::Failed("Not signed in".into())));
    }
}
