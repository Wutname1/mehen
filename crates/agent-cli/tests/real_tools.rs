//! Runs against the Claude Code and Codex installed on this computer. Ignored
//! because they need the tool installed and signed in, and spend a little of
//! the person's quota. Run by hand:
//!
//! `cargo test -p agent-cli --test real_tools -- --ignored --nocapture`

use std::sync::Mutex;
use std::time::Duration;

use agent_cli::{Availability, CancelToken, Event, Permissions, Task, Tool, detect, run};

async fn says_ok(tool: Tool) {
    let found = detect(tool).await;
    eprintln!("{tool}: {:?} {:?} at {:?}", found.availability, found.version, found.program);
    assert_eq!(found.availability, Availability::Ready, "{tool} is not usable on this computer");

    let cwd = std::env::temp_dir().join(format!("agent-cli-real-{}", serde_json_name(tool)));
    std::fs::create_dir_all(&cwd).unwrap();
    let task = Task {
        tool,
        cwd,
        prompt: "Reply with the word ok and nothing else. Do not use any tools.".into(),
        system_prompt: None,
        permissions: Permissions { edit_files: false, run_commands: false },
        model: None,
        timeout: Some(Duration::from_secs(180)),
    };
    let events = Mutex::new(Vec::new());
    let outcome = run(task, CancelToken::new(), |event| {
        eprintln!("{event:?}");
        events.lock().unwrap().push(event);
    })
    .await
    .unwrap_or_else(|e| panic!("{tool} run failed: {e}"));
    eprintln!("{tool}: {outcome:?}");
    assert!(outcome.finished, "{outcome:?}");
    assert!(outcome.summary.to_lowercase().contains("ok"), "{outcome:?}");
    let events = events.into_inner().unwrap();
    assert!(events.iter().any(|e| matches!(e, Event::Text { .. })), "no text arrived: {events:?}");
    assert!(events.iter().any(|e| matches!(e, Event::Usage { .. })), "no usage arrived: {events:?}");
}

fn serde_json_name(tool: Tool) -> &'static str {
    match tool {
        Tool::Claude => "claude",
        Tool::Codex => "codex",
    }
}

#[tokio::test]
#[ignore]
async fn claude_answers_a_trivial_read_only_task() {
    says_ok(Tool::Claude).await;
}

#[tokio::test]
#[ignore]
async fn codex_answers_a_trivial_read_only_task() {
    says_ok(Tool::Codex).await;
}
