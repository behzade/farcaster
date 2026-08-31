use std::{
    collections::HashSet,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::{
    agents::{
        AgentLaunchConfig, FileAccessMode, NetworkAccessMode, PermissionLevel, SessionCommand,
        SessionEvent, SessionLaunch, SessionStart, SessionTransport,
        extensions::{ExtensionUiRequest, ExtensionUiResponse, PromptMode},
    },
    app::views::transcript::conversation::{ConversationState, TranscriptKind},
};

use super::{
    delete_external_session, farcaster_mcp, load_external_history,
    main_session::external_session_locator, spawn_session,
};

const TURN_TIMEOUT: Duration = Duration::from_secs(180);

struct McpGuard;

impl McpGuard {
    fn disabled() -> Self {
        farcaster_mcp::set_enabled(false);
        Self
    }
}

impl Drop for McpGuard {
    fn drop(&mut self) {
        farcaster_mcp::set_enabled(true);
    }
}

#[test]
#[ignore = "runs Codex and OpenCode against their configured live LLM accounts"]
fn live_harnesses_conform_to_session_outcomes() -> Result<(), String> {
    let _mcp = McpGuard::disabled();
    let selected = std::env::var("FARCASTER_E2E_HARNESS").ok();
    for harness in ["codex-cli", "opencode2"] {
        if selected
            .as_deref()
            .is_some_and(|selected| selected != harness)
        {
            continue;
        }
        exercise_live_harness(harness)
            .map_err(|error| format!("{harness} live conformance failed: {error}"))?;
    }
    Ok(())
}

fn exercise_live_harness(harness: &str) -> Result<(), String> {
    let project =
        std::env::current_dir().map_err(|error| format!("resolve live-test project: {error}"))?;
    let locator_root = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is required for live harness tests".to_owned())?
        .join(".local/share/farcaster/session-locators");
    let config = AgentLaunchConfig {
        program: PathBuf::from(harness),
        prefix_args: Vec::new(),
        permission_level: PermissionLevel {
            files: FileAccessMode::Full,
            network: NetworkAccessMode::Full,
        },
        sandbox: crate::access::test_sandbox_bypass(),
        grants: None,
        app_proxy: None,
        session_locator_root: Some(locator_root),
    };
    let launch = |start, session_id| SessionLaunch {
        harness: harness.to_owned(),
        session_id,
        project: project.clone(),
        start,
        wake: None,
    };
    let mut session = spawn_session(&config, launch(SessionStart::New, None))?;
    let path = session_path(&mut *session)?;
    let result = exercise_live_session(&mut *session).and_then(|()| {
        session
            .send(SessionCommand::Rename {
                name: "Farcaster live conformance".into(),
            })
            .map(|_| ())
            .map_err(|error| format!("rename failed: {error}"))
    });
    let result = result.and(session.close());
    let result = result.and_then(|()| {
        let history = load_external_history(&path)
            .ok_or_else(|| "live session did not use an external backend locator".to_owned())?
            .map_err(|error| format!("history load failed: {error}"))?;
        if history
            .messages
            .iter()
            .any(|message| message.to_string().contains("FARCASTER_FOLLOWUP_OK"))
        {
            Ok(())
        } else {
            Err("persisted history omitted the final follow-up response".into())
        }
    });
    let result = result.and_then(|()| {
        let locator = external_session_locator(harness, &path)
            .ok_or_else(|| format!("invalid live session path: {}", path.display()))?;
        let mut resumed = spawn_session(
            &config,
            launch(SessionStart::Resume(path.clone()), Some(locator)),
        )
        .map_err(|error| format!("resume failed: {error}"))?;
        resumed
            .close()
            .map_err(|error| format!("resumed session close failed: {error}"))
    });
    let delete = delete_external_session(&path)
        .ok_or_else(|| "live session did not expose backend deletion".to_owned())?
        .map_err(|error| format!("delete failed: {error}"));
    result.and(delete)
}

fn session_path(session: &mut dyn SessionTransport) -> Result<PathBuf, String> {
    session.send(SessionCommand::LoadState)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        match session.poll() {
            Some(SessionEvent::Response(response)) if response.command == "get_state" => {
                return response
                    .data
                    .get("sessionFile")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .ok_or_else(|| "session state omitted its locator path".to_owned());
            }
            Some(SessionEvent::Failure(error)) => return Err(error),
            Some(_) => {}
            None => thread::sleep(Duration::from_millis(20)),
        }
    }
    Err("timed out loading the live session locator".into())
}

fn exercise_live_session(session: &mut dyn SessionTransport) -> Result<(), String> {
    let mut conversation = ConversationState::default();
    let mut observed_usage = None;
    let mut tool_starts = HashSet::new();
    let mut tool_ends = HashSet::new();
    let mut steered = false;
    session.send(SessionCommand::Prompt {
        mode: PromptMode::Normal,
        message: "You must use the shell tool to run exactly `sleep 2; printf FARCASTER_TOOL_OK`. After it finishes, reply with FARCASTER_BASE_OK.".into(),
        images: Vec::new(),
    })?;
    poll_until(
        session,
        &mut conversation,
        TURN_TIMEOUT,
        |session, event, _| {
            capture_usage(event, &mut observed_usage);
            if event.get("type").and_then(Value::as_str) == Some("tool_execution_start") {
                if let Some(id) = event.get("toolCallId").and_then(Value::as_str) {
                    tool_starts.insert(id.to_owned());
                }
                if !steered {
                    session.send(SessionCommand::Prompt {
                        mode: PromptMode::Steer,
                        message:
                            "Your final response must include the exact token FARCASTER_STEER_OK."
                                .into(),
                        images: Vec::new(),
                    })?;
                    steered = true;
                }
            }
            if event.get("type").and_then(Value::as_str) == Some("tool_execution_end")
                && let Some(id) = event.get("toolCallId").and_then(Value::as_str)
            {
                tool_ends.insert(id.to_owned());
            }
            Ok(event.get("type").and_then(Value::as_str) == Some("agent_settled"))
        },
    )?;
    if !steered {
        return Err("the harness never emitted a tool start, so steering was not exercised".into());
    }
    if tool_starts.is_empty() || !tool_starts.is_subset(&tool_ends) {
        return Err(format!(
            "tool lifecycle was incomplete: starts={tool_starts:?}, ends={tool_ends:?}"
        ));
    }
    require_assistant_text(&conversation, "FARCASTER_STEER_OK")?;
    require_usage(observed_usage.as_ref())?;

    let mut follow_up_sent = false;
    session.send(SessionCommand::Prompt {
        mode: PromptMode::Normal,
        message: "Use the shell tool to run `sleep 2`; then reply with FARCASTER_QUEUE_BASE_OK."
            .into(),
        images: Vec::new(),
    })?;
    poll_until(
        session,
        &mut conversation,
        TURN_TIMEOUT,
        |session, event, conversation| {
            capture_usage(event, &mut observed_usage);
            if event.get("type").and_then(Value::as_str) == Some("agent_start") && !follow_up_sent {
                session.send(SessionCommand::Prompt {
                    mode: PromptMode::FollowUp,
                    message:
                        "After the current turn, reply with the exact token FARCASTER_FOLLOWUP_OK."
                            .into(),
                    images: Vec::new(),
                })?;
                follow_up_sent = true;
            }
            Ok(
                conversation_contains(&conversation, "FARCASTER_FOLLOWUP_OK")
                    && event.get("type").and_then(Value::as_str) == Some("agent_settled"),
            )
        },
    )?;
    if !follow_up_sent {
        return Err("the harness settled before a follow-up could be queued".into());
    }
    require_assistant_text(&conversation, "FARCASTER_FOLLOWUP_OK")?;
    require_usage(observed_usage.as_ref())?;
    if conversation.average_cache_hit_rate.is_none() {
        return Err("assistant finalization omitted cache usage".into());
    }
    require_usage_response(session)?;

    session.send(SessionCommand::Compact { instructions: None })?;
    let mut compaction_started = false;
    poll_until(session, &mut conversation, TURN_TIMEOUT, |_, event, _| {
        match event.get("type").and_then(Value::as_str) {
            Some("compaction_start") => compaction_started = true,
            Some("compaction_end") if compaction_started => return Ok(true),
            _ => {}
        }
        Ok(false)
    })?;
    Ok(())
}

fn capture_usage(event: &Value, observed: &mut Option<Value>) {
    if event.get("type").and_then(Value::as_str) == Some("turn_end") {
        *observed = Some(event.clone());
    }
}

fn require_usage(event: Option<&Value>) -> Result<(), String> {
    let event = event.ok_or_else(|| "backend emitted no normalized token usage".to_owned())?;
    let usage = event
        .get("usage")
        .ok_or_else(|| "turn usage is missing".to_owned())?;
    for key in ["input", "output", "cacheRead", "cacheWrite", "totalTokens"] {
        if usage.get(key).and_then(Value::as_u64).is_none() {
            return Err(format!("turn usage omitted {key}"));
        }
    }
    if usage.get("input").and_then(Value::as_u64).unwrap_or(0) == 0
        || usage.get("output").and_then(Value::as_u64).unwrap_or(0) == 0
    {
        return Err(format!("turn usage is empty: {usage}"));
    }
    if event
        .get("contextWindow")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        == 0
    {
        return Err("backend omitted the model context window".into());
    }
    Ok(())
}

fn require_usage_response(session: &mut dyn SessionTransport) -> Result<(), String> {
    session.send(SessionCommand::LoadUsage)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        match session.poll() {
            Some(SessionEvent::Response(response)) if response.command == "get_session_stats" => {
                require_usage(Some(&json!({
                    "type": "turn_end",
                    "contextWindow": response.data.pointer("/contextUsage/contextWindow"),
                    "usage": response.data.get("tokens"),
                })))?;
                let percent = response
                    .data
                    .pointer("/contextUsage/percent")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                return (percent > 0.0)
                    .then_some(())
                    .ok_or_else(|| "context usage percentage was not populated".to_owned());
            }
            Some(SessionEvent::Failure(error)) => return Err(error),
            Some(_) => {}
            None => thread::sleep(Duration::from_millis(20)),
        }
    }
    Err("timed out loading normalized token usage".into())
}

fn poll_until(
    session: &mut dyn SessionTransport,
    conversation: &mut ConversationState,
    timeout: Duration,
    mut done: impl FnMut(&mut dyn SessionTransport, &Value, &ConversationState) -> Result<bool, String>,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let Some(item) = session.poll() else {
            thread::sleep(Duration::from_millis(20));
            continue;
        };
        match item {
            SessionEvent::Activity(event) => {
                conversation.reduce(&event);
                if done(session, &event, conversation)? {
                    return Ok(());
                }
            }
            SessionEvent::Interaction(request) => approve(session, request)?,
            SessionEvent::Failure(error) => return Err(error),
            SessionEvent::Response(_) | SessionEvent::Stderr(_) => {}
        }
    }
    Err(format!("timed out after {} seconds", timeout.as_secs()))
}

fn approve(session: &mut dyn SessionTransport, request: ExtensionUiRequest) -> Result<(), String> {
    let response = match request {
        ExtensionUiRequest::Select { id, options, .. } => ExtensionUiResponse::Value {
            id,
            value: options.into_iter().next().unwrap_or_else(|| "Allow".into()),
        },
        ExtensionUiRequest::Confirm { id, .. } => ExtensionUiResponse::Confirmed {
            id,
            confirmed: true,
        },
        ExtensionUiRequest::Input { id, .. } | ExtensionUiRequest::Editor { id, .. } => {
            ExtensionUiResponse::Value {
                id,
                value: "Allow".into(),
            }
        }
        _ => return Ok(()),
    };
    session.respond(response)
}

fn conversation_contains(conversation: &ConversationState, expected: &str) -> bool {
    conversation.items.iter().any(|item| {
        item.kind == TranscriptKind::Assistant && item.complete_text().contains(expected)
    })
}

fn require_assistant_text(conversation: &ConversationState, expected: &str) -> Result<(), String> {
    if conversation_contains(conversation, expected) {
        return Ok(());
    }
    let assistant = conversation
        .items
        .iter()
        .filter(|item| item.kind == TranscriptKind::Assistant)
        .map(|item| item.complete_text())
        .collect::<Vec<_>>();
    Err(format!(
        "assistant transcript does not contain {expected:?}: {assistant:?}"
    ))
}
