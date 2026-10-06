//! Driving Claude Code through its print-mode JSON stream.
//!
//! `claude --print --input-format stream-json --output-format stream-json
//! --verbose` keeps a conversation open on stdin and writes one JSON object
//! per line on stdout. (`--verbose` is required with stream-json output or the
//! CLI refuses to start.) The shapes below are undocumented for hosts other
//! than the official SDK; they were read out of the CLI (2.1.251) and checked
//! again against 2.1.287:
//!
//! - in: `{"type":"user","message":{"role":"user","content":[{"type":"text",
//!   "text":...}]}}`, and `control_request` frames of our own (`initialize`,
//!   `interrupt`).
//! - out: `assistant` frames carrying `text` and `tool_use` blocks, `user`
//!   frames carrying each `tool_result`, and one `result` frame per turn.
//! - with `--permission-prompt-tool stdio`, each prompt arrives as
//!   `{"type":"control_request","request_id":ID,"request":{"subtype":
//!   "can_use_tool","tool_name":..,"input":{..},"tool_use_id":..}}` and waits
//!   for `{"type":"control_response","response":{"subtype":"success",
//!   "request_id":ID,"response":{"behavior":"allow","updatedInput":{..}}}}` or
//!   `{"behavior":"deny","message":..}`. Any other `control_request` gets an
//!   error frame, or the CLI waits on it forever.
//!
//! Prompts are not the only bound, because they have not always arrived: on
//! 2.1.260 a `mkdir` ran with no prompt at all (upstream claude-code#34046).
//! On 2.1.287 they arrive again. So what is not allowed is also kept off the
//! command line: `--restricted` confines file tools to the working folder and
//! drops command and web tools unless `--tools` names them, and
//! `--disallowedTools` removes the rest before the process starts.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::permissions::{self, Kind, Verdict};
use crate::process::{self, Launch, Running};
use crate::{CancelToken, Error, Event, Outcome, Permissions, Task};

/// How long a cancelled run gets to wind its turn down before it is stopped.
const INTERRUPT_GRACE: Duration = Duration::from_secs(2);

const READ_TOOLS: &[&str] = &["Read", "Glob", "Grep", "LS", "NotebookRead", "TodoWrite"];
const EDIT_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit"];
/// `BashOutput`/`KillShell` were renamed `TaskStop` and friends in later
/// versions; names the CLI does not have are ignored, so both are listed.
const COMMAND_TOOLS: &[&str] = &["Bash", "BashOutput", "KillShell", "TaskStop", "PowerShell", "Monitor"];
const NETWORK_TOOLS: &[&str] = &["WebFetch", "WebSearch"];

fn kind(tool: &str) -> Kind {
    if READ_TOOLS.contains(&tool) {
        Kind::Read
    } else if EDIT_TOOLS.contains(&tool) {
        Kind::Edit
    } else if COMMAND_TOOLS.contains(&tool) {
        Kind::Command
    } else if NETWORK_TOOLS.contains(&tool) {
        Kind::Network
    } else {
        Kind::Other
    }
}

fn edited_path(input: &Value) -> Option<&str> {
    input.get("file_path").or_else(|| input.get("notebook_path")).and_then(Value::as_str)
}

fn judge(tool: &str, input: &Value, cwd: &Path, permissions: Permissions) -> Verdict {
    let paths: Vec<String> = edited_path(input).map(str::to_string).into_iter().collect();
    permissions::judge(kind(tool), tool, &paths, cwd, permissions)
}

/// Every flag for one run.
///
/// Naming any tool in `--tools` replaces the whole set rather than adding to
/// it, so the read tools are always named too. `--strict-mcp-config` keeps
/// the person's own MCP servers (mail, calendars, ...) out of an unattended
/// run that nothing here could judge; it also keeps the prompt small.
/// `--no-session-persistence` keeps these runs out of their session history.
fn launch_args(task: &Task) -> Vec<String> {
    let p = task.permissions;
    let mut allowed: Vec<&str> = READ_TOOLS.to_vec();
    let mut denied: Vec<&str> = Vec::new();
    for (tools, allow) in [(EDIT_TOOLS, p.edit_files), (COMMAND_TOOLS, p.run_commands), (NETWORK_TOOLS, p.run_commands)] {
        if allow { allowed.extend(tools) } else { denied.extend(tools) }
    }
    let mut args: Vec<String> = ["--print", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"].map(String::from).to_vec();
    args.extend(["--permission-mode", "default", "--permission-prompt-tool", "stdio", "--restricted", "--strict-mcp-config", "--no-session-persistence"].map(String::from));
    args.push("--tools".into());
    args.push(allowed.join(","));
    if !denied.is_empty() {
        args.push("--disallowedTools".into());
        args.push(denied.join(","));
    }
    if let Some(model) = task.model.as_deref().filter(|m| !m.trim().is_empty()) {
        args.push("--model".into());
        args.push(model.to_string());
    }
    args
}

/// The opening control request. The system prompt rides here rather than on
/// the command line (`--append-system-prompt`): measured on 2.1.287, it is
/// applied the same, and a long prompt with newlines never has to survive
/// Windows quoting through a `.cmd` shim.
fn initialize(task: &Task) -> Value {
    let mut request = json!({ "subtype": "initialize" });
    if let Some(prompt) = task.system_prompt.as_deref().filter(|p| !p.trim().is_empty()) {
        request["appendSystemPrompt"] = json!(prompt);
    }
    control_request("agent-cli-init", request)
}

fn control_request(id: &str, request: Value) -> Value {
    json!({ "type": "control_request", "request_id": id, "request": request })
}

fn user_message(text: &str) -> Value {
    json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "text", "text": text }] } })
}

fn permission_answer(request_id: &Value, verdict: &Verdict, input: &Value) -> Value {
    let response = match verdict {
        // The SDK echoes the input back as `updatedInput`; nothing here
        // rewrites what the model asked for.
        Verdict::Allow => json!({ "behavior": "allow", "updatedInput": input }),
        // The model reads this, so it can choose another route instead of
        // retrying the same call.
        Verdict::Deny(reason) => json!({ "behavior": "deny", "message": format!("Not allowed: {reason}. Do not retry it unchanged; find another way or say plainly that you cannot.") }),
    };
    json!({ "type": "control_response", "response": { "subtype": "success", "request_id": request_id, "response": response } })
}

fn control_error(request_id: &Value) -> Value {
    json!({ "type": "control_response", "response": { "subtype": "error", "request_id": request_id, "error": "This host does not handle this request" } })
}

/// One line describing a tool use, for a person.
fn title(tool: &str, input: &Value, cwd: &Path) -> String {
    let field = |name: &str| input.get(name).and_then(Value::as_str);
    let shorten = |text: &str| if text.chars().count() > 120 { format!("{}...", text.chars().take(117).collect::<String>()) } else { text.to_string() };
    match kind(tool) {
        Kind::Command if field("command").is_some() => format!("Run {}", shorten(field("command").unwrap_or_default().trim())),
        Kind::Edit => format!("Edit {}", edited_path(input).map(|p| permissions::display_path(cwd, p)).unwrap_or_else(|| "a file".into())),
        _ => match tool {
            "Read" => format!("Read {}", field("file_path").map(|p| permissions::display_path(cwd, p)).unwrap_or_else(|| "a file".into())),
            "Glob" | "Grep" => format!("Search {}", shorten(field("pattern").unwrap_or("files"))),
            "WebFetch" => format!("Fetch {}", shorten(field("url").unwrap_or("a page"))),
            "WebSearch" => format!("Search the web for {}", shorten(field("query").unwrap_or("something"))),
            _ => tool.to_string(),
        },
    }
}

struct ToolUse {
    name: String,
    input: Value,
    title: String,
}

/// What one line from the CLI asks of the run.
#[derive(Default)]
struct Step {
    events: Vec<Event>,
    reply: Option<Value>,
    done: Option<Result<Outcome, Error>>,
}

/// Turns the CLI's lines into events and answers, remembering what it needs
/// across lines (which tool each result belongs to, what was already denied).
struct Session {
    cwd: std::path::PathBuf,
    permissions: Permissions,
    tools: HashMap<String, ToolUse>,
    denied: HashSet<String>,
    changed: HashSet<String>,
    last_text: String,
}

impl Session {
    fn new(task: &Task) -> Self {
        Session { cwd: task.cwd.clone(), permissions: task.permissions, tools: HashMap::new(), denied: HashSet::new(), changed: HashSet::new(), last_text: String::new() }
    }

    fn handle(&mut self, msg: &Value) -> Step {
        let mut step = Step::default();
        match msg.get("type").and_then(Value::as_str) {
            Some("control_request") => self.control_request(msg, &mut step),
            Some("assistant") => self.assistant(msg, &mut step),
            Some("user") => self.tool_results(msg, &mut step),
            Some("result") => self.result(msg, &mut step),
            _ => {}
        }
        step
    }

    fn control_request(&mut self, msg: &Value, step: &mut Step) {
        let Some(request_id) = msg.get("request_id") else { return };
        let request = msg.get("request").unwrap_or(&Value::Null);
        if request.get("subtype").and_then(Value::as_str) != Some("can_use_tool") {
            step.reply = Some(control_error(request_id));
            return;
        }
        let tool = request.get("tool_name").and_then(Value::as_str).unwrap_or_default();
        let input = request.get("input").cloned().unwrap_or(Value::Null);
        let verdict = judge(tool, &input, &self.cwd, self.permissions);
        if let Verdict::Deny(reason) = &verdict {
            if let Some(id) = request.get("tool_use_id").and_then(Value::as_str) {
                self.denied.insert(id.to_string());
            }
            step.events.push(Event::Denied { title: title(tool, &input, &self.cwd), reason: reason.clone() });
        }
        step.reply = Some(permission_answer(request_id, &verdict, &input));
    }

    fn assistant(&mut self, msg: &Value, step: &mut Step) {
        let Some(blocks) = msg.pointer("/message/content").and_then(Value::as_array) else { return };
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let text = block.get("text").and_then(Value::as_str).unwrap_or_default();
                    if !text.is_empty() {
                        self.last_text = text.to_string();
                        step.events.push(Event::Text { text: text.to_string() });
                    }
                }
                Some("tool_use") => {
                    let id = block.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                    let name = block.get("name").and_then(Value::as_str).unwrap_or("Tool").to_string();
                    let input = block.get("input").cloned().unwrap_or(Value::Null);
                    let title = title(&name, &input, &self.cwd);
                    step.events.push(Event::ToolCall { id: id.clone(), title: title.clone(), status: "running".into() });
                    self.tools.insert(id, ToolUse { name, input, title });
                }
                _ => {}
            }
        }
    }

    fn tool_results(&mut self, msg: &Value, step: &mut Step) {
        let Some(blocks) = msg.pointer("/message/content").and_then(Value::as_array) else { return };
        for block in blocks.iter().filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result")) {
            let id = block.get("tool_use_id").and_then(Value::as_str).unwrap_or_default();
            let failed = block.get("is_error").and_then(Value::as_bool).unwrap_or(false);
            let Some(tool) = self.tools.get(id) else { continue };
            step.events.push(Event::ToolCall { id: id.to_string(), title: tool.title.clone(), status: if failed { "failed" } else { "done" }.into() });
            if failed {
                // A tool kept off the command line fails as "no such tool"
                // without any prompt, so it is reported as a denial here.
                if let (Verdict::Deny(reason), false) = (judge(&tool.name, &tool.input, &self.cwd, self.permissions), self.denied.contains(id)) {
                    self.denied.insert(id.to_string());
                    step.events.push(Event::Denied { title: tool.title.clone(), reason });
                }
            } else if kind(&tool.name) == Kind::Edit
                && let Some(path) = edited_path(&tool.input).map(|p| permissions::display_path(&self.cwd, p))
                    && self.changed.insert(path.clone()) {
                        step.events.push(Event::FileChanged { path });
                    }
        }
    }

    fn result(&mut self, msg: &Value, step: &mut Step) {
        // Refusals the CLI made by itself, without asking.
        for denial in msg.get("permission_denials").and_then(Value::as_array).into_iter().flatten() {
            let id = denial.get("tool_use_id").and_then(Value::as_str).unwrap_or_default();
            if !self.denied.insert(id.to_string()) {
                continue;
            }
            let tool = denial.get("tool_name").and_then(Value::as_str).unwrap_or("Tool");
            let input = denial.get("tool_input").cloned().unwrap_or(Value::Null);
            let reason = match judge(tool, &input, &self.cwd, self.permissions) {
                Verdict::Deny(reason) => reason,
                Verdict::Allow => "Claude Code refused it".into(),
            };
            step.events.push(Event::Denied { title: title(tool, &input, &self.cwd), reason });
        }
        if let Some(usage) = msg.get("usage") {
            let count = |name: &str| usage.get(name).and_then(Value::as_u64);
            let cached = count("cache_creation_input_tokens").unwrap_or(0) + count("cache_read_input_tokens").unwrap_or(0);
            step.events.push(Event::Usage { input_tokens: count("input_tokens").map(|n| n + cached), output_tokens: count("output_tokens") });
        }
        step.done = Some(self.outcome(msg));
    }

    fn outcome(&self, msg: &Value) -> Result<Outcome, Error> {
        let text = msg.get("result").and_then(Value::as_str).filter(|t| !t.trim().is_empty());
        let subtype = msg.get("subtype").and_then(Value::as_str).unwrap_or("success");
        let is_error = msg.get("is_error").and_then(Value::as_bool).unwrap_or(false);
        let summary = text.map(str::to_string).unwrap_or_else(|| self.last_text.clone());
        match subtype {
            // An API error (not signed in, out of quota) is a `success`
            // frame with `is_error` set and the reason as its text.
            "success" if is_error => Err(Error::Failed(text.unwrap_or("Claude Code could not finish this task").to_string())),
            "success" => {
                let stop = msg.get("stop_reason").and_then(Value::as_str);
                Ok(Outcome { finished: matches!(stop, None | Some("end_turn") | Some("stop_sequence")), summary, stop_reason: stop.map(str::to_string) })
            }
            // It ran out of turns or budget: work stopped short, not failed.
            s if s.starts_with("error_max") => Ok(Outcome { finished: false, summary, stop_reason: Some(s.to_string()) }),
            other => {
                let errors: Vec<&str> = msg.get("errors").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
                Err(Error::Failed(if errors.is_empty() { text.map(str::to_string).unwrap_or_else(|| format!("Claude Code stopped: {other}")) } else { errors.join("\n") }))
            }
        }
    }
}

pub(crate) async fn run(launch: &Launch, task: &Task, cancel: &CancelToken, on_event: &(impl Fn(Event) + Send + Sync)) -> Result<Outcome, Error> {
    let mut running = process::spawn(launch, &launch_args(task), &task.cwd)?;
    let mut session = Session::new(task);
    let work = drive(&mut running, &mut session, task, on_event);
    let stopped = process::until_stopped(work, cancel, task.timeout).await;
    match stopped {
        process::Stopped::Done(result) => {
            process::finish(&mut running.child, &mut running.stdin).await;
            result
        }
        process::Stopped::Cancelled => {
            // An interrupt lets the CLI end its turn and stop a running
            // command itself; the tree is stopped regardless after a moment.
            let interrupt = control_request("agent-cli-interrupt", json!({ "subtype": "interrupt" }));
            if process::write_line(&mut running.stdin, &interrupt).await.is_ok() {
                let _ = tokio::time::timeout(INTERRUPT_GRACE, async {
                    while let Ok(Some(line)) = running.lines.next_line().await {
                        if line.contains("\"type\":\"result\"") {
                            break;
                        }
                    }
                })
                .await;
            }
            process::stop(&mut running.child).await;
            Err(Error::Cancelled)
        }
        process::Stopped::TimedOut => {
            process::stop(&mut running.child).await;
            Err(Error::TimedOut)
        }
    }
}

async fn drive(running: &mut Running, session: &mut Session, task: &Task, on_event: &(impl Fn(Event) + Send + Sync)) -> Result<Outcome, Error> {
    let send_failed = |e: std::io::Error| Error::Spawn(format!("Could not send the task to Claude Code: {e}"));
    process::write_line(&mut running.stdin, &initialize(task)).await.map_err(send_failed)?;
    process::write_line(&mut running.stdin, &user_message(&task.prompt)).await.map_err(send_failed)?;
    while let Ok(Some(line)) = running.lines.next_line().await {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let step = session.handle(&msg);
        for event in step.events {
            on_event(event);
        }
        if let Some(reply) = step.reply {
            // A prompt left unanswered would hold the turn forever.
            process::write_line(&mut running.stdin, &reply).await.map_err(|e| Error::Failed(running.stderr.explain(&format!("Claude Code stopped listening: {e}"))))?;
        }
        if let Some(done) = step.done {
            return done;
        }
    }
    Err(Error::Failed(running.stderr.explain("Claude Code stopped unexpectedly")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(permissions: Permissions) -> Task {
        Task { tool: crate::Tool::Claude, cwd: std::env::temp_dir().join("agent-cli-claude-test"), prompt: "fix it".into(), system_prompt: None, permissions, model: None, timeout: None }
    }

    fn after(flag: &str, args: &[String]) -> Option<String> {
        args.iter().position(|a| a == flag).map(|i| args[i + 1].clone())
    }

    fn line(text: &str) -> Value {
        serde_json::from_str(text).expect("a recorded line")
    }

    #[test]
    fn launch_flags_keep_what_is_not_allowed_out_of_the_process() {
        let args = launch_args(&task(Permissions::default()));
        for needed in ["--print", "--verbose", "--restricted", "--strict-mcp-config", "--permission-prompt-tool"] {
            assert!(args.contains(&needed.to_string()), "{needed} missing from {args:?}");
        }
        assert_eq!(after("--input-format", &args).as_deref(), Some("stream-json"));
        assert_eq!(after("--output-format", &args).as_deref(), Some("stream-json"));
        assert_eq!(after("--permission-mode", &args).as_deref(), Some("default"), "not acceptEdits: edits must reach the prompt so their paths are checked");
        let tools = after("--tools", &args).unwrap();
        let denied = after("--disallowedTools", &args).unwrap();
        for read in ["Read", "Glob", "Grep"] {
            assert!(tools.split(',').any(|t| t == read), "naming tools replaces the set, so {read} must be named");
        }
        for refused in ["Write", "Edit", "MultiEdit", "NotebookEdit", "Bash", "PowerShell", "WebFetch", "WebSearch"] {
            assert!(denied.split(',').any(|t| t == refused), "{refused} must be denied at launch: {denied}");
            assert!(!tools.split(',').any(|t| t == refused), "{refused} must not be named back: {tools}");
        }
    }

    #[test]
    fn allowed_tools_are_named_back_and_nothing_is_denied() {
        let args = launch_args(&Task { model: Some("haiku".into()), ..task(Permissions { edit_files: true, run_commands: true }) });
        let tools = after("--tools", &args).unwrap();
        for tool in ["Edit", "Write", "Bash", "PowerShell", "WebFetch"] {
            assert!(tools.split(',').any(|t| t == tool), "{tool} must be named back after --restricted: {tools}");
        }
        assert!(!args.contains(&"--disallowedTools".to_string()));
        assert_eq!(after("--model", &args).as_deref(), Some("haiku"));
    }

    #[test]
    fn the_system_prompt_rides_in_initialize() {
        let with = initialize(&Task { system_prompt: Some("Be brief.\nVery.".into()), ..task(Permissions::default()) });
        assert_eq!(with["type"], "control_request");
        assert_eq!(with["request"]["subtype"], "initialize");
        assert_eq!(with["request"]["appendSystemPrompt"], "Be brief.\nVery.");
        assert!(initialize(&task(Permissions::default()))["request"].get("appendSystemPrompt").is_none());
    }

    /// Recorded from 2.1.287: a `mkdir` the model wanted to run.
    #[test]
    fn a_command_prompt_is_answered_from_permissions() {
        let frame = line(r#"{"type":"control_request","request_id":"c891190a-9d13-43ad-a71e-a3ea32d3fde6","request":{"subtype":"can_use_tool","tool_name":"Bash","display_name":"Bash","input":{"command":"mkdir made-by-bash"},"description":"mkdir made-by-bash","tool_use_id":"toolu_01F7xzAJrGamDk2KRamKVaT2"}}"#);
        let mut refused = Session::new(&task(Permissions::default()));
        let step = refused.handle(&frame);
        let reply = step.reply.expect("answered");
        assert_eq!(reply["type"], "control_response");
        assert_eq!(reply["response"]["subtype"], "success");
        assert_eq!(reply["response"]["request_id"], "c891190a-9d13-43ad-a71e-a3ea32d3fde6");
        assert_eq!(reply["response"]["response"]["behavior"], "deny");
        assert_eq!(step.events, vec![Event::Denied { title: "Run mkdir made-by-bash".into(), reason: permissions::NO_COMMANDS.into() }]);

        let mut allowed = Session::new(&task(Permissions { edit_files: false, run_commands: true }));
        let reply = allowed.handle(&frame).reply.unwrap();
        assert_eq!(reply["response"]["response"]["behavior"], "allow");
        assert_eq!(reply["response"]["response"]["updatedInput"], json!({ "command": "mkdir made-by-bash" }));
    }

    #[test]
    fn an_edit_outside_the_folder_is_refused_even_when_edits_are_allowed() {
        let t = task(Permissions { edit_files: true, run_commands: false });
        let elsewhere = std::env::temp_dir().join("elsewhere.txt").to_string_lossy().into_owned();
        let frame = json!({ "type": "control_request", "request_id": "r2", "request": { "subtype": "can_use_tool", "tool_name": "Write", "input": { "file_path": elsewhere, "content": "x" }, "tool_use_id": "t2" } });
        let step = Session::new(&t).handle(&frame);
        assert_eq!(step.reply.unwrap()["response"]["response"]["behavior"], "deny");
        let inside = json!({ "type": "control_request", "request_id": "r3", "request": { "subtype": "can_use_tool", "tool_name": "Edit", "input": { "file_path": t.cwd.join("src").join("lib.rs") }, "tool_use_id": "t3" } });
        let step = Session::new(&t).handle(&inside);
        assert_eq!(step.reply.unwrap()["response"]["response"]["behavior"], "allow");
        assert!(step.events.is_empty());
    }

    #[test]
    fn other_control_requests_are_declined_not_ignored() {
        let step = Session::new(&task(Permissions::default())).handle(&json!({ "type": "control_request", "request_id": "h1", "request": { "subtype": "hook_callback" } }));
        let reply = step.reply.unwrap();
        assert_eq!(reply["response"]["subtype"], "error");
        assert_eq!(reply["response"]["request_id"], "h1");
    }

    /// Recorded from 2.1.287 with Bash kept off the command line: the model
    /// tries it anyway and the CLI answers "no such tool" without a prompt.
    #[test]
    fn a_tool_denied_at_launch_is_reported_when_the_model_tries_it() {
        let mut session = Session::new(&task(Permissions::default()));
        let used = session.handle(&line(r#"{"type":"assistant","message":{"model":"claude-haiku-4-5-20251001","id":"msg_011CfkY42iAsos63czoVYiB1","type":"message","role":"assistant","content":[{"type":"tool_use","id":"toolu_01Xiaibj2BEXVDeKod5Hm2Tj","name":"Bash","input":{"command":"mkdir zz"},"caller":{"type":"direct"}}],"stop_reason":null},"parent_tool_use_id":null,"session_id":"9e0873a2"}"#));
        assert_eq!(used.events, vec![Event::ToolCall { id: "toolu_01Xiaibj2BEXVDeKod5Hm2Tj".into(), title: "Run mkdir zz".into(), status: "running".into() }]);
        let result = session.handle(&line(r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"<tool_use_error>Error: No such tool available: Bash. Bash is disabled for this session, in subagents as well as here.</tool_use_error>","is_error":true,"tool_use_id":"toolu_01Xiaibj2BEXVDeKod5Hm2Tj"}]},"parent_tool_use_id":null,"session_id":"9e0873a2"}"#));
        assert_eq!(result.events[0], Event::ToolCall { id: "toolu_01Xiaibj2BEXVDeKod5Hm2Tj".into(), title: "Run mkdir zz".into(), status: "failed".into() });
        assert_eq!(result.events[1], Event::Denied { title: "Run mkdir zz".into(), reason: permissions::NO_COMMANDS.into() });
    }

    #[test]
    fn a_denial_is_reported_once_however_many_frames_mention_it() {
        let mut session = Session::new(&task(Permissions::default()));
        session.handle(&json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "id": "t1", "name": "Bash", "input": { "command": "ls" } }] } }));
        let asked = session.handle(&json!({ "type": "control_request", "request_id": "r", "request": { "subtype": "can_use_tool", "tool_name": "Bash", "input": { "command": "ls" }, "tool_use_id": "t1" } }));
        assert_eq!(asked.events.len(), 1);
        let failed = session.handle(&json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1", "is_error": true, "content": "denied" }] } }));
        assert!(!failed.events.iter().any(|e| matches!(e, Event::Denied { .. })));
        let done = session.handle(&json!({ "type": "result", "subtype": "success", "is_error": false, "result": "ok", "permission_denials": [{ "tool_name": "Bash", "tool_use_id": "t1", "tool_input": { "command": "ls" } }] }));
        assert!(!done.events.iter().any(|e| matches!(e, Event::Denied { .. })));
    }

    #[test]
    fn text_and_successful_edits_become_events() {
        let t = task(Permissions { edit_files: true, run_commands: false });
        let mut session = Session::new(&t);
        let path = t.cwd.join("src").join("lib.rs").to_string_lossy().into_owned();
        let step = session.handle(&json!({ "type": "assistant", "message": { "content": [
            { "type": "text", "text": "Fixing the import." },
            { "type": "tool_use", "id": "e1", "name": "Edit", "input": { "file_path": path, "old_string": "a", "new_string": "b" } }
        ] } }));
        assert_eq!(step.events[0], Event::Text { text: "Fixing the import.".into() });
        assert_eq!(step.events[1], Event::ToolCall { id: "e1".into(), title: "Edit src/lib.rs".into(), status: "running".into() });
        let done = session.handle(&json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "tool_use_id": "e1", "is_error": false, "content": "ok" }] } }));
        assert_eq!(done.events, vec![Event::ToolCall { id: "e1".into(), title: "Edit src/lib.rs".into(), status: "done".into() }, Event::FileChanged { path: "src/lib.rs".into() }]);
    }

    /// Trimmed from a recorded 2.1.287 `result` frame.
    #[test]
    fn a_successful_result_ends_the_run_with_usage() {
        let mut session = Session::new(&task(Permissions::default()));
        let step = session.handle(&line(r#"{"duration_api_ms":2531,"stop_reason":"end_turn","session_id":"9e0873a2","total_cost_usd":0.0049473,"usage":{"input_tokens":18,"cache_creation_input_tokens":1376,"cache_read_input_tokens":11973,"output_tokens":196},"permission_denials":[],"terminal_reason":"completed","is_error":false,"num_turns":2,"subtype":"success","api_error_status":null,"result":"refused","type":"result","duration_ms":3445}"#));
        assert_eq!(step.events, vec![Event::Usage { input_tokens: Some(18 + 1376 + 11973), output_tokens: Some(196) }]);
        assert_eq!(step.done.unwrap().unwrap(), Outcome { finished: true, summary: "refused".into(), stop_reason: Some("end_turn".into()) });
    }

    #[test]
    fn errors_and_limits_end_the_run_differently() {
        let session = Session::new(&task(Permissions::default()));
        let signed_out = session.outcome(&json!({ "type": "result", "subtype": "success", "is_error": true, "result": "Not logged in · Please run /login" }));
        assert_eq!(signed_out, Err(Error::Failed("Not logged in · Please run /login".into())));
        let limited = session.outcome(&json!({ "type": "result", "subtype": "error_max_turns", "is_error": true }));
        assert!(!limited.unwrap().finished);
        let long = session.outcome(&json!({ "type": "result", "subtype": "success", "stop_reason": "max_tokens", "result": "partial" }));
        assert!(!long.unwrap().finished);
        let broke = session.outcome(&json!({ "type": "result", "subtype": "error_during_execution", "is_error": true, "errors": ["boom"] }));
        assert_eq!(broke, Err(Error::Failed("boom".into())));
    }
}
