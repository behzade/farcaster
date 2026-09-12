use super::super::main_session::{WorkerSessionTransport, external_session_path};
use crate::agents::{
    AgentLaunchConfig, HarnessAccessMode, SessionCommand, SessionEvent, SessionLaunch,
    SessionStart, SessionTransport,
    extensions::{ExtensionUiRequest, ExtensionUiResponse, PromptMode},
};
use crate::app::views::transcript::conversation::{ConversationState, TranscriptKind};
use std::io::Write as _;
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn setup(project: &Path, access_mode: HarnessAccessMode) -> (AgentLaunchConfig, SessionLaunch) {
    (
        AgentLaunchConfig {
            program: super::program(),
            prefix_args: Vec::new(),
            access_mode,
            app_proxy: None,
            session_locator_root: Some(project.join("locators")),
        },
        SessionLaunch {
            harness: super::BACKEND.into(),
            session_id: None,
            project: project.into(),
            start: SessionStart::New,
            wake: None,
        },
    )
}

struct Turn {
    conversation: ConversationState,
    agent_started: bool,
}

fn has_image(conversation: &ConversationState) -> bool {
    conversation
        .items
        .iter()
        .any(|item| item.kind == TranscriptKind::User && item.images.len() == 1)
}

fn turn(
    session: &mut dyn SessionTransport,
    prompt: String,
    expected: &str,
    tool: bool,
) -> Result<Turn, String> {
    session.send(SessionCommand::Prompt {
        mode: PromptMode::Normal,
        message: prompt,
        images: Vec::new(),
    })?;
    settled(session, expected, tool)
}

fn settled(session: &mut dyn SessionTransport, expected: &str, tool: bool) -> Result<Turn, String> {
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut conversation = ConversationState::default();
    let mut streamed = false;
    let mut tool_finished = false;
    let mut agent_started = false;
    let mut usage = false;
    while Instant::now() < deadline {
        match session.poll() {
            Some(SessionEvent::Activity(event)) => {
                conversation.reduce(event.value());
                streamed |= event.value()["assistantMessageEvent"]["type"] == "text_delta";
                tool_finished |= event.value()["type"] == "tool_execution_end";
                agent_started |= event.value()["type"] == "tool_execution_start"
                    && matches!(event.value()["toolName"].as_str(), Some("Agent" | "Task"));
                if event.value()["type"] == "turn_end" {
                    usage |= event.value()["usage"]["input"].as_u64().unwrap_or(0) > 0
                        && event.value()["usage"]["output"].as_u64().unwrap_or(0) > 0;
                }
                if event.value()["type"] == "agent_settled" {
                    let found = conversation.items.iter().any(|item| {
                        item.kind == TranscriptKind::Assistant
                            && item.complete_text().contains(expected)
                            && !item.complete_text().contains("No response requested.")
                    });
                    if !found || !streamed || !usage || (tool && !tool_finished) {
                        return Err(format!(
                            "turn did not render expected text/tool output: found={found}, streamed={streamed}, usage={usage}, tool_finished={tool_finished}"
                        ));
                    }
                    if !expected.is_empty() {
                        let occurrences: usize = conversation
                            .items
                            .iter()
                            .filter(|item| item.kind == TranscriptKind::Assistant)
                            .map(|item| item.complete_text().matches(expected).count())
                            .sum();
                        if occurrences != 1 {
                            return Err(format!(
                                "answer appeared {occurrences} times instead of once"
                            ));
                        }
                    }
                    return Ok(Turn {
                        conversation,
                        agent_started,
                    });
                }
            }
            Some(SessionEvent::Interaction(ExtensionUiRequest::Select { id, options, .. })) => {
                if !options.iter().any(|option| option == "Allow") {
                    return Err("unexpected permission choices".into());
                }
                session.respond(ExtensionUiResponse::Value {
                    id,
                    value: "Allow".into(),
                })?;
            }
            Some(SessionEvent::Failure(error)) => return Err(error),
            Some(SessionEvent::Response(crate::agents::SessionResponse {
                result: Err(error),
                ..
            })) => {
                return Err(error.to_string());
            }
            _ => thread::sleep(Duration::from_millis(10)),
        }
    }
    Err("live Claude turn timed out".into())
}

#[test]
#[ignore = "uses the installed Claude CLI and real account; leaves a native test session"]
fn real_claude_text_followup_tool_and_resume() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let (config, mut launch) = setup(project.path(), HarnessAccessMode::Sandboxed);
    let (worker, id, metadata) = super::spawn_main(&config, &launch)?;
    let root = config
        .session_locator_root
        .as_deref()
        .expect("test operation should succeed");
    let path = external_session_path(root, super::BACKEND, &id);
    let mut session =
        WorkerSessionTransport::new(root, super::BACKEND, id.clone(), worker, metadata, None)?;
    writeln!(std::io::stderr().lock(), "Claude live session {id}").expect("write test diagnostics");
    turn(&mut session, "hi".into(), "", false)?;
    writeln!(
        std::io::stderr().lock(),
        "literal hi prompt rendered a streamed answer with usage"
    )
    .expect("write test diagnostics");
    session.send(SessionCommand::Prompt {mode:PromptMode::Normal,
        message:"Look at this image, then reply exactly LIVE_IMAGE_OK.".into(),
        images:vec![crate::protocol::PromptImage::new("iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAIAAAD8GO2jAAAAKklEQVR4nGP4EKBBU8QwasGoBaMWjFowasGoBaMWjFowasGoBaMWDBULACvxoEydbL2eAAAAAElFTkSuQmCC".into(), "image/png".into())]})?;
    let image_turn = settled(&mut session, "LIVE_IMAGE_OK", false)?;
    if !has_image(&image_turn.conversation) {
        return Err("attached image disappeared from live transcript".into());
    }
    writeln!(
        std::io::stderr().lock(),
        "attached image remained visible and answer appeared exactly once"
    )
    .expect("write test diagnostics");
    std::fs::write(project.path().join("proof.txt"), "LIVE_READ_OK")
        .map_err(|error| error.to_string())?;
    turn(
        &mut session,
        "Use Read to read proof.txt, then reply with its exact contents.".into(),
        "LIVE_READ_OK",
        true,
    )?;
    writeln!(
        std::io::stderr().lock(),
        "real Read tool completed and rendered"
    )
    .expect("write test diagnostics");
    session.send(SessionCommand::Prompt {
        mode: PromptMode::Normal,
        message: "Reply exactly LIVE_QUEUE_FIRST. Do not use tools.".into(),
        images: Vec::new(),
    })?;
    session.send(SessionCommand::Prompt {
        mode: PromptMode::FollowUp,
        message: "Reply exactly LIVE_QUEUE_SECOND. Do not use tools.".into(),
        images: Vec::new(),
    })?;
    settled(&mut session, "LIVE_QUEUE_FIRST", false)?;
    settled(&mut session, "LIVE_QUEUE_SECOND", false)?;
    writeln!(
        std::io::stderr().lock(),
        "queued prompt delivered after first turn settled; nonzero usage verified on each turn"
    )
    .expect("write test diagnostics");
    session.send(SessionCommand::Prompt {
        mode: PromptMode::Steer,
        message: "Reply exactly LIVE_STEER_OK. Do not use tools.".into(),
        images: Vec::new(),
    })?;
    session.send(SessionCommand::ApplySteering)?;
    settled(&mut session, "LIVE_STEER_OK", false)?;
    writeln!(
        std::io::stderr().lock(),
        "applied steering was admitted and delivered"
    )
    .expect("write test diagnostics");
    let child_turn = turn(&mut session,"Use the Agent tool to ask a subagent to read proof.txt and return its contents. After that subagent returns LIVE_READ_OK, reply LIVE_SUBAGENT_OK. Do not read the file yourself.".into(),"LIVE_SUBAGENT_OK",true)?;
    if !child_turn.agent_started {
        return Err("model answered without invoking a native subagent".into());
    }
    writeln!(
        std::io::stderr().lock(),
        "native subagent tool completed and parent answer rendered"
    )
    .expect("write test diagnostics");
    session.close()?;
    let history = super::load_history(&path)?;
    let mut saved = ConversationState::default();
    saved.replace_history(&history.messages);
    if !has_image(&saved) {
        return Err("attached image disappeared from reloaded history".into());
    }
    if !history
        .messages
        .iter()
        .any(|message| message.to_string().contains("LIVE_IMAGE_OK"))
    {
        return Err("native history missed completed turn".into());
    }
    launch.start = SessionStart::Resume(path);
    let mut resumed = super::super::spawn_session(&config, launch)?;
    turn(
        &mut *resumed,
        "Reply exactly LIVE_RESUME_OK. Do not use tools.".into(),
        "LIVE_RESUME_OK",
        false,
    )?;
    resumed.close()?;
    writeln!(
        std::io::stderr().lock(),
        "history loaded and resumed turn rendered and settled"
    )
    .expect("write test diagnostics");
    Ok(())
}

#[test]
#[ignore = "uses the installed Claude CLI and real account; leaves native test sessions"]
fn real_claude_interrupt_and_recover() -> Result<(), String> {
    use crate::agents::{WorkerActivity, WorkerEvent, WorkerSendMode};
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let (config, launch) = setup(project.path(), HarnessAccessMode::Full);
    for during_tool in [false, true] {
        let (mut worker, id, _) = super::spawn_main(&config, &launch)?;
        writeln!(
            std::io::stderr().lock(),
            "interrupt session {id}: during_tool={during_tool}"
        )
        .expect("write test diagnostics");
        worker.send(
            "Use Bash to run sleep 30 in the foreground (run_in_background=false), then reply LIVE_SHOULD_BE_INTERRUPTED.".into(),
            WorkerSendMode::Prompt,
        )?;
        worker.send(
            "Reply QUEUED_MUST_BE_CANCELLED".into(),
            WorkerSendMode::Queue,
        )?;
        let mut aborted = false;
        let mut finished = false;
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            if let Some(event) = worker.poll() {
                let trigger = if during_tool {
                    matches!(&event, WorkerEvent::Activity(WorkerActivity::ToolStarted {name,..}) if name == "Bash")
                } else {
                    matches!(event, WorkerEvent::Started)
                };
                if trigger && !aborted {
                    worker.abort()?;
                    aborted = true;
                }
                match event {
                    WorkerEvent::Failed(error) => return Err(format!("interrupt failed: {error}")),
                    WorkerEvent::Settled { .. } => {
                        finished = true;
                        break;
                    }
                    _ => {}
                }
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }
        if !aborted || !finished {
            worker.close()?;
            return Err(format!(
                "interrupt failed to settle: aborted={aborted}, finished={finished}, during_tool={during_tool}"
            ));
        }
        worker.send(
            "Reply exactly LIVE_RECOVER_OK. Do not use tools.".into(),
            WorkerSendMode::Prompt,
        )?;
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut recovered = false;
        while Instant::now() < deadline {
            match worker.poll() {
                Some(WorkerEvent::Settled { output }) => {
                    recovered = output.contains("LIVE_RECOVER_OK")
                        && !output.contains("QUEUED_MUST_BE_CANCELLED");
                    break;
                }
                Some(WorkerEvent::Failed(error)) => return Err(error),
                _ => thread::sleep(Duration::from_millis(10)),
            }
        }
        worker.close()?;
        if !recovered {
            return Err("session did not recover after interrupt".into());
        }
        writeln!(
            std::io::stderr().lock(),
            "interrupt, queue cancellation, and recovery passed: during_tool={during_tool}"
        )
        .expect("write test diagnostics");
    }
    Ok(())
}
