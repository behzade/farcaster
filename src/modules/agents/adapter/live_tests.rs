use crate::agents::{SessionHistory, SessionResponsePayload as Payload};
use std::io::Write as _;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::{
    agents::{
        AgentLaunchConfig, HarnessAccessMode, SessionCommand, SessionEvent, SessionLaunch,
        SessionOperation, SessionResponse, SessionStart, SessionTransport,
        extensions::{ExtensionUiRequest, ExtensionUiResponse, PromptImage, PromptMode},
    },
    app::views::transcript::conversation::{ConversationState, TranscriptKind},
};

use super::super::contract::{AgentBackendDescriptor, AgentCapabilities, CapabilitySupport};
use super::{
    delete_external_session, farcaster_mcp, known_backend_descriptors, load_external_history,
    main_session::external_session_locator, spawn_session,
};

const TURN_TIMEOUT: Duration = Duration::from_secs(180);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const LIVE_HARNESSES: [&str; 6] = [
    "pi",
    "codex-cli",
    "cursor-cli",
    "opencode2",
    "claude",
    "antigravity-acp",
];
const TEST_IMAGE: &str = "iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAIAAAD8GO2jAAAAKklEQVR4nGP4EKBBU8QwasGoBaMWjFowasGoBaMWjFowasGoBaMWDBULACvxoEydbL2eAAAAAElFTkSuQmCC";

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Coverage {
    history: bool,
    usage: bool,
    streamed_text: bool,
    tool_activity: bool,
    models: bool,
    select_model: bool,
    reasoning: bool,
    modes: bool,
    commands: bool,
    images: bool,
    abort: bool,
    steer: bool,
    follow_up: bool,
    queue: bool,
    compact: bool,
    rename: bool,
    resume: bool,
    move_project: bool,
    delete: bool,
}

impl Coverage {
    fn from_capabilities(capabilities: &AgentCapabilities) -> Self {
        let available = |support: &CapabilitySupport| *support == CapabilitySupport::Available;
        Self {
            history: available(&capabilities.sessions.history),
            usage: available(&capabilities.observation.usage),
            streamed_text: available(&capabilities.observation.streamed_text),
            tool_activity: available(&capabilities.observation.tool_activity),
            models: available(&capabilities.configuration.models),
            select_model: available(&capabilities.configuration.select_model),
            reasoning: available(&capabilities.configuration.reasoning_effort),
            modes: available(&capabilities.configuration.modes),
            commands: available(&capabilities.configuration.commands),
            images: available(&capabilities.turns.images),
            abort: available(&capabilities.turns.interrupt),
            steer: available(&capabilities.turns.steer),
            follow_up: available(&capabilities.turns.follow_up),
            queue: available(&capabilities.turns.queue),
            compact: available(&capabilities.turns.compact),
            rename: available(&capabilities.sessions.rename),
            resume: available(&capabilities.sessions.resume),
            move_project: available(&capabilities.sessions.move_project),
            delete: available(&capabilities.sessions.delete),
        }
    }
}

fn select_harnesses(selected: Option<&str>) -> Result<Vec<&'static str>, String> {
    let Some(selected) = selected else {
        return Ok(LIVE_HARNESSES.to_vec());
    };
    LIVE_HARNESSES
        .into_iter()
        .find(|harness| *harness == selected)
        .map(|harness| vec![harness])
        .ok_or_else(|| {
            format!(
                "unknown FARCASTER_E2E_HARNESS {selected:?}; expected one of {}",
                LIVE_HARNESSES.join(", ")
            )
        })
}

fn descriptor(harness: &str) -> Result<AgentBackendDescriptor, String> {
    known_backend_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.id.as_str() == harness)
        .ok_or_else(|| format!("live harness {harness} has no backend descriptor"))
}

#[test]
#[ignore = "runs all six backends against live LLM accounts; consumes usage and retains sessions when deletion is unsupported"]
fn live_harnesses_conform_to_session_outcomes() -> Result<(), String> {
    let _mcp = McpGuard::disabled();
    let selected = std::env::var("FARCASTER_E2E_HARNESS").ok();
    for harness in select_harnesses(selected.as_deref())? {
        let descriptor = descriptor(harness)?;
        exercise_live_harness(harness, &descriptor.capabilities)
            .map_err(|error| format!("{harness} live conformance failed: {error}"))?;
    }
    Ok(())
}

fn exercise_live_harness(harness: &str, capabilities: &AgentCapabilities) -> Result<(), String> {
    let coverage = Coverage::from_capabilities(capabilities);
    let project_guard =
        tempfile::tempdir_in(std::env::current_dir().map_err(|error| error.to_string())?)
            .map_err(|error| format!("create isolated live-test project: {error}"))?;
    let project = project_guard
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let locator_root = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is required for live harness tests".to_owned())?
        .join(".local/share/farcaster/session-locators");
    let config = AgentLaunchConfig {
        program: PathBuf::from(harness),
        prefix_args: Vec::new(),
        access_mode: HarnessAccessMode::Full,
        app_proxy: None,
        session_locator_root: Some(locator_root),
    };
    let launch = |start, session_id| SessionLaunch {
        harness: harness.to_owned(),
        session_id,
        project: project.clone(),
        start,
        wake: Some(thread::current()),
    };
    let mut session = spawn_session(&config, launch(SessionStart::New, None))?;
    let path = session_path(&mut *session)?;
    if !coverage.delete {
        writeln!(
            std::io::stderr().lock(),
            "{harness}: live test session {} will remain because deletion is unsupported",
            path.display()
        )
        .expect("write test diagnostics");
    }

    let outcome = (|| {
        exercise_catalog(&mut *session, coverage)?;
        let marker = exercise_live_session(&mut *session, coverage)?;
        if coverage.abort {
            exercise_abort(&mut *session)?;
        }
        if coverage.compact {
            exercise_compaction(&mut *session)?;
        }
        if coverage.rename {
            request(
                &mut *session,
                SessionCommand::Rename {
                    name: "Farcaster live conformance".into(),
                },
            )?;
        }
        if coverage.history && harness == "pi" {
            require_history_response(&mut *session, &marker)?;
        }
        if coverage.usage {
            require_usage_response(&mut *session)?;
        }
        Ok::<_, String>(marker)
    })();
    let close = session.close();
    let marker = match outcome {
        Ok(marker) => marker,
        Err(error) => return Err(cleanup_error(error, close, harness, &path, coverage)),
    };
    close.map_err(|error| cleanup_error(error, Ok(()), harness, &path, coverage))?;
    if coverage.move_project {
        exercise_live_move(harness, &config, &project, &path, &marker, coverage)
            .map_err(|error| cleanup_error(error, Ok(()), harness, &path, coverage))?;
    }
    verify_persistence_and_cleanup(harness, &config, &launch, &path, &marker, coverage)
        .map_err(|error| cleanup_error(error, Ok(()), harness, &path, coverage))
}

fn exercise_live_move(
    harness: &str,
    config: &AgentLaunchConfig,
    source: &Path,
    path: &Path,
    marker: &str,
    coverage: Coverage,
) -> Result<(), String> {
    let destination_guard =
        tempfile::tempdir_in(std::env::current_dir().map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let destination = destination_guard
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let discover =
        || super::discover_sessions_for(harness, config.session_locator_root.as_deref(), "");
    let original = discover()?
        .into_iter()
        .find(|session| session.path == path)
        .ok_or("move fixture missing from catalog")?;
    let rediscover = |project: &Path, path: &Path| {
        let mut matches = discover()?
            .into_iter()
            .filter(|session| {
                session.id == original.id
                    || session.project == destination
                    || session.project == source
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(format!("move duplicated or lost the fixture: {matches:?}"));
        }
        let stored = matches.remove(0);
        if stored.id != original.id
            || stored.path != path
            || stored.project != project
            || stored.parent_session != original.parent_session
            || (harness != "pi" && stored.path != original.path)
        {
            return Err(format!(
                "move changed identity or retained the old project: {stored:?}"
            ));
        }
        Ok(stored)
    };
    let mut current = original.clone();
    let outcome = (|| {
        // Move back as well, so persistence checks and cleanup use the original locator.
        for project in [destination.as_path(), source] {
            let moved = super::move_session_family(&[current.clone()], project)?;
            current.path = moved.root.clone();
            if moved.paths.len() != 1 {
                return Err("move returned an invalid locator mapping".into());
            }
            current = rediscover(project, &moved.root)?;
            let history = super::load_session_history(harness, &current.path)?;
            if !history
                .messages
                .iter()
                .any(|message| message.to_string().contains(marker))
            {
                return Err("move lost the existing conversation".into());
            }
            let mut resumed = spawn_session(
                config,
                SessionLaunch {
                    harness: harness.into(),
                    session_id: Some(original.id.clone()),
                    project: project.into(),
                    start: SessionStart::Resume(current.path.clone()),
                    wake: Some(thread::current()),
                },
            )?;
            let check = (|| {
                if session_path(&mut *resumed)? != current.path {
                    return Err("resume created a session".into());
                }
                require_history_response(&mut *resumed, marker)?;
                // The prompt does not disclose the token or an absolute path. A real
                // relative file read must use the moved session's working directory.
                let token = format!(
                    "MOVE_CWD_{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .expect("test operation should succeed")
                        .as_nanos()
                );
                fs::write(project.join("move-cwd-proof.txt"), &token)
                    .map_err(|error| error.to_string())?;
                resumed.send(SessionCommand::Prompt {
                    mode: PromptMode::Normal,
                    message: "Read move-cwd-proof.txt in your current working directory using a tool. Reply with its exact contents. Do not search other directories or change directory.".into(),
                    images: Vec::new(),
                })?;
                let mut conversation = ConversationState::default();
                poll_until(
                    &mut *resumed,
                    &mut conversation,
                    &mut Lifecycle::default(),
                    &mut HashMap::new(),
                    TURN_TIMEOUT,
                    |_, event, _| Ok(event["type"].as_str() == Some("agent_settled")),
                )?;
                require_assistant_text(&conversation, &token)
            })();
            let close = resumed.close();
            check?;
            close?;
            rediscover(project, &current.path)?;
        }
        Ok(())
    })();
    if outcome.is_err() && current.path != path {
        cleanup_failed_fixture(harness, &current.path, coverage)?;
    }
    outcome
}

fn cleanup_error(
    error: String,
    close: Result<(), String>,
    harness: &str,
    path: &Path,
    coverage: Coverage,
) -> String {
    let details = [
        close.err(),
        cleanup_failed_fixture(harness, path, coverage).err(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if details.is_empty() {
        error
    } else {
        format!("{error}; cleanup failed: {}", details.join("; "))
    }
}

fn exercise_catalog(session: &mut dyn SessionTransport, coverage: Coverage) -> Result<(), String> {
    request(session, SessionCommand::ConfigureSteering)?;
    let Payload::LoadState(state) = request(session, SessionCommand::LoadState)? else {
        return Err("expected state response".into());
    };
    if coverage.history {
        request(session, SessionCommand::LoadHistory)?;
    }
    if coverage.models {
        let Payload::ListModels(models) = request(session, SessionCommand::ListModels)? else {
            return Err("expected model catalog".into());
        };
        if models.is_empty() {
            return Err("live model catalog is empty".into());
        }
        if coverage.select_model {
            let model = state
                .model
                .as_ref()
                .or_else(|| models.first())
                .ok_or_else(|| "live model catalog is empty".to_owned())?;
            if model.context_window != 0 {
                request(
                    session,
                    SessionCommand::SelectModel {
                        provider: model.provider.clone(),
                        model_id: model.id.clone(),
                    },
                )?;
            }
        }
    }
    if coverage.reasoning {
        let Payload::ListReasoningLevels(levels) =
            request(session, SessionCommand::ListReasoningLevels)?
        else {
            return Err("expected effort catalog".into());
        };
        if let Some(level) = levels.first() {
            request(
                session,
                SessionCommand::SelectReasoning {
                    level: level.clone(),
                },
            )?;
        }
    }
    if coverage.modes {
        let Payload::ListModes { modes, .. } = request(session, SessionCommand::ListModes)? else {
            return Err("expected mode catalog".into());
        };
        if let Some(mode) = modes.first() {
            request(
                session,
                SessionCommand::SelectMode {
                    mode: mode.id.clone(),
                },
            )?;
        }
    }
    if coverage.commands
        && !matches!(
            request(session, SessionCommand::ListCommands)?,
            Payload::ListCommands(_)
        )
    {
        return Err("expected command catalog".into());
    }
    Ok(())
}

fn exercise_live_session(
    session: &mut dyn SessionTransport,
    coverage: Coverage,
) -> Result<String, String> {
    let marker = format!("FARCASTER_LIVE_{}", std::process::id());
    let final_marker = if coverage.follow_up {
        "FARCASTER_FOLLOWUP_OK"
    } else if coverage.steer {
        "FARCASTER_STEER_OK"
    } else {
        marker.as_str()
    };
    let mut conversation = ConversationState::default();
    let mut lifecycle = Lifecycle::default();
    let mut responses = HashMap::new();
    let mut steered = false;
    let mut steer_id = None;
    let message = if coverage.tool_activity {
        format!(
            "The attached image is a test fixture. You must use the shell tool to run exactly `sleep 2; printf {marker}`. After it finishes, reply with the exact token {marker}."
        )
    } else {
        format!("Reply with the exact token {marker}.")
    };
    session.send(SessionCommand::Prompt {
        mode: PromptMode::Normal,
        message,
        images: if coverage.images {
            vec![PromptImage::new(TEST_IMAGE.into(), "image/png".into())]
        } else {
            Vec::new()
        },
    })?;
    poll_until(
        session,
        &mut conversation,
        &mut lifecycle,
        &mut responses,
        TURN_TIMEOUT,
        |session, event, _| {
            if event.get("type").and_then(Value::as_str) == Some("tool_execution_start")
                && coverage.steer
                && !steered
            {
                let id = session.send(SessionCommand::Prompt {
                    mode: PromptMode::Steer,
                    message:
                        "Your final response must also include the exact token FARCASTER_STEER_OK."
                            .into(),
                    images: Vec::new(),
                })?;
                steer_id = Some(id);
                steered = true;
            }
            Ok(event.get("type").and_then(Value::as_str) == Some("agent_settled"))
        },
    )?;
    lifecycle.require_turn(
        coverage.usage,
        coverage.queue && coverage.steer,
        coverage.streamed_text,
        coverage.tool_activity,
    )?;
    require_assistant_text(
        &conversation,
        if coverage.steer {
            "FARCASTER_STEER_OK"
        } else {
            &marker
        },
    )?;
    if let Some(id) = steer_id {
        require_response(&responses, &id, SessionOperation::Prompt(PromptMode::Steer))?;
    }

    if coverage.follow_up {
        let mut follow_up_sent = false;
        let mut follow_up_id = None;
        let mut follow_lifecycle = Lifecycle::default();
        session.send(SessionCommand::Prompt {
            mode: PromptMode::Normal,
            message:
                "Use the shell tool to run `sleep 2`; then reply with FARCASTER_QUEUE_BASE_OK."
                    .into(),
            images: Vec::new(),
        })?;
        poll_until(
            session,
            &mut conversation,
            &mut follow_lifecycle,
            &mut responses,
            TURN_TIMEOUT,
            |session, event, conversation| {
                if event.get("type").and_then(Value::as_str) == Some("agent_start")
                    && !follow_up_sent
                {
                    let id = session.send(SessionCommand::Prompt {
                        mode: PromptMode::FollowUp,
                        message:
                            "After the current turn, reply with the exact token FARCASTER_FOLLOWUP_OK."
                                .into(),
                        images: Vec::new(),
                    })?;
                    follow_up_id = Some(id);
                    follow_up_sent = true;
                }
                Ok(conversation_contains(conversation, "FARCASTER_FOLLOWUP_OK")
                    && event.get("type").and_then(Value::as_str) == Some("agent_settled"))
            },
        )?;
        follow_lifecycle.require_turn(
            coverage.usage,
            coverage.queue,
            coverage.streamed_text,
            coverage.tool_activity,
        )?;
        let id = follow_up_id.ok_or_else(|| "follow-up was not queued".to_owned())?;
        require_response(
            &responses,
            &id,
            SessionOperation::Prompt(PromptMode::FollowUp),
        )?;
        require_assistant_text(&conversation, final_marker)?;
    }
    Ok(final_marker.into())
}

#[derive(Default)]
struct Lifecycle {
    types: Vec<String>,
    tool_starts: HashSet<String>,
    tool_ends: HashSet<String>,
    saw_usage: bool,
    saw_queue: bool,
}

impl Lifecycle {
    fn observe(&mut self, event: &Value) {
        let Some(kind) = event.get("type").and_then(Value::as_str) else {
            return;
        };
        self.types.push(kind.into());
        match kind {
            "tool_execution_start" => {
                if let Some(id) = event.get("toolCallId").and_then(Value::as_str) {
                    self.tool_starts.insert(id.into());
                }
            }
            "tool_execution_end" => {
                if let Some(id) = event.get("toolCallId").and_then(Value::as_str) {
                    self.tool_ends.insert(id.into());
                }
            }
            "turn_end" => self.saw_usage = true,
            "queue_update" => self.saw_queue = true,
            _ => {}
        }
    }

    fn require_turn(
        &self,
        usage: bool,
        queue: bool,
        streamed_text: bool,
        tool_activity: bool,
    ) -> Result<(), String> {
        self.require_order("agent_start", "agent_settled")?;
        if streamed_text {
            self.require_order("message_start", "message_update")?;
            self.require_order("message_update", "message_end")?;
        }
        if tool_activity
            && (self.tool_starts.is_empty() || !self.tool_starts.is_subset(&self.tool_ends))
        {
            return Err(format!(
                "tool lifecycle was incomplete: starts={:?}, ends={:?}",
                self.tool_starts, self.tool_ends
            ));
        }
        if usage && !self.saw_usage {
            return Err("turn lifecycle omitted normalized usage".into());
        }
        if queue && !self.saw_queue {
            return Err("queued delivery omitted queue_update".into());
        }
        Ok(())
    }

    fn require_order(&self, start: &str, end: &str) -> Result<(), String> {
        let start_index = self.types.iter().position(|kind| kind == start);
        let end_index = self.types.iter().rposition(|kind| kind == end);
        if start_index
            .zip(end_index)
            .is_some_and(|(start, end)| start < end)
        {
            Ok(())
        } else {
            Err(format!("invalid {start}/{end} lifecycle: {:?}", self.types))
        }
    }
}

fn exercise_abort(session: &mut dyn SessionTransport) -> Result<(), String> {
    session.send(SessionCommand::Prompt {
        mode: PromptMode::Normal,
        message: "Use the shell tool to run exactly `sleep 30`; do not do anything else.".into(),
        images: Vec::new(),
    })?;
    let deadline = Instant::now() + TURN_TIMEOUT;
    let mut conversation = ConversationState::default();
    let mut abort_id = None;
    let mut abort_response = false;
    let mut started = false;
    let mut settled = false;
    while Instant::now() < deadline && !(abort_response && settled) {
        match session.poll() {
            Some(SessionEvent::Activity(event)) => {
                conversation.reduce(event.value());
                let kind = event.value().get("type").and_then(Value::as_str);
                started |= kind == Some("agent_start");
                if kind == Some("agent_start") && abort_id.is_none() {
                    abort_id = Some(session.send(SessionCommand::Abort)?);
                }
                settled |= abort_id.is_some() && kind == Some("agent_settled");
            }
            Some(SessionEvent::Response(response)) => {
                if abort_id.as_deref() == response.id.as_deref() {
                    if let Err(error) = &response.result {
                        return Err(error.to_string());
                    }
                    abort_response = response.operation() == SessionOperation::Abort;
                }
            }
            Some(SessionEvent::Interaction(request)) => approve(session, request)?,
            Some(SessionEvent::Failure(error)) => return Err(error),
            Some(SessionEvent::Stderr(_)) | None => thread::sleep(Duration::from_millis(20)),
        }
    }
    if !started || abort_id.is_none() || !abort_response || !settled {
        return Err(format!(
            "abort lifecycle incomplete: started={started}, sent={}, response={abort_response}, settled={settled}",
            abort_id.is_some()
        ));
    }
    Ok(())
}

fn exercise_compaction(session: &mut dyn SessionTransport) -> Result<(), String> {
    let id = match session.send(SessionCommand::Compact { instructions: None }) {
        Ok(id) => id,
        Err(error) if compaction_not_needed(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    let deadline = Instant::now() + TURN_TIMEOUT;
    let mut started = false;
    let mut finished = false;
    let mut response = false;
    while Instant::now() < deadline && !(started && finished && response) {
        match session.poll() {
            Some(SessionEvent::Activity(event)) => {
                match event.value().get("type").and_then(Value::as_str) {
                    Some("compaction_start") => started = true,
                    Some("compaction_end") if started => finished = true,
                    _ => {}
                }
            }
            Some(SessionEvent::Response(item)) if item.id.as_deref() == Some(&id) => {
                if let Err(error) = &item.result {
                    let error = error.to_string();
                    return if compaction_not_needed(&error) {
                        Ok(())
                    } else {
                        Err(error)
                    };
                }
                response = item.operation() == SessionOperation::Compact;
            }
            Some(SessionEvent::Interaction(request)) => approve(session, request)?,
            Some(SessionEvent::Failure(error)) if compaction_not_needed(&error) => return Ok(()),
            Some(SessionEvent::Failure(error)) => return Err(error),
            Some(SessionEvent::Response(_) | SessionEvent::Stderr(_)) | None => {
                thread::sleep(Duration::from_millis(20));
            }
        }
    }
    if started && finished && response {
        Ok(())
    } else {
        Err(format!(
            "compaction lifecycle incomplete: start={started}, end={finished}, response={response}"
        ))
    }
}

fn compaction_not_needed(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    [
        "nothing to compact",
        "session too small",
        "no messages to compact",
    ]
    .into_iter()
    .any(|message| error.contains(message))
}

fn cleanup_failed_fixture(harness: &str, path: &Path, coverage: Coverage) -> Result<(), String> {
    if harness == "pi" {
        return if path.is_file() {
            fs::remove_file(path)
                .map_err(|error| format!("delete Pi session {}: {error}", path.display()))
        } else {
            Ok(())
        };
    }
    if coverage.delete {
        delete_external_session(path)
            .ok_or_else(|| "external backend omitted deletion".to_owned())?
            .map_err(|error| format!("delete failed: {error}"))
    } else {
        Ok(())
    }
}

fn verify_persistence_and_cleanup(
    harness: &str,
    config: &AgentLaunchConfig,
    launch: &impl Fn(SessionStart, Option<String>) -> SessionLaunch,
    path: &Path,
    marker: &str,
    coverage: Coverage,
) -> Result<(), String> {
    if harness == "pi" {
        if coverage.history {
            let contents = fs::read_to_string(path).map_err(|error| {
                format!("read persisted Pi session {}: {error}", path.display())
            })?;
            if !contents.contains(marker) {
                return Err("persisted Pi history omitted the final response".into());
            }
        }
        if coverage.resume {
            let mut resumed =
                spawn_session(config, launch(SessionStart::Resume(path.into()), None))?;
            if coverage.history {
                require_history_response(&mut *resumed, marker)?;
            }
            resumed.close()?;
        }
        if coverage.delete {
            fs::remove_file(path)
                .map_err(|error| format!("delete Pi session {}: {error}", path.display()))?;
        }
        return Ok(());
    }

    if coverage.history {
        let history = load_external_history(path)
            .ok_or_else(|| "live session did not use an external backend locator".to_owned())?
            .map_err(|error| format!("history load failed: {error}"))?;
        if !history
            .messages
            .iter()
            .any(|message| message.to_string().contains(marker))
        {
            return Err("persisted external history omitted the final response".into());
        }
    }
    if coverage.resume {
        let locator = external_session_locator(harness, path)
            .ok_or_else(|| format!("invalid live session path: {}", path.display()))?;
        let mut resumed = spawn_session(
            config,
            launch(SessionStart::Resume(path.into()), Some(locator)),
        )?;
        if coverage.history {
            require_history_response(&mut *resumed, marker)?;
        }
        resumed.close()?;
    }
    if coverage.delete {
        delete_external_session(path)
            .ok_or_else(|| "external backend omitted deletion".to_owned())?
            .map_err(|error| format!("delete failed: {error}"))
    } else {
        Ok(())
    }
}

fn session_path(session: &mut dyn SessionTransport) -> Result<PathBuf, String> {
    let Payload::LoadState(state) = request(session, SessionCommand::LoadState)? else {
        return Err("expected state response".into());
    };
    state
        .session_file
        .map(PathBuf::from)
        .ok_or_else(|| "session state omitted its locator path".to_owned())
}

fn request(session: &mut dyn SessionTransport, command: SessionCommand) -> Result<Payload, String> {
    let operation = command.response_operation();
    let id = session.send(command)?;
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    while Instant::now() < deadline {
        match session.poll() {
            Some(SessionEvent::Response(response)) if response.id.as_deref() == Some(&id) => {
                if response.operation() != operation {
                    return Err(format!(
                        "command {id} returned {:?}, expected {operation:?}",
                        response.operation()
                    ));
                }
                return response.result.map_err(|error| error.to_string());
            }
            Some(SessionEvent::Interaction(request)) => approve(session, request)?,
            Some(SessionEvent::Failure(error)) => return Err(error),
            Some(_) | None => thread::sleep(Duration::from_millis(20)),
        }
    }
    Err(format!("timed out waiting for {operation:?}"))
}

fn require_history_response(
    session: &mut dyn SessionTransport,
    expected: &str,
) -> Result<(), String> {
    let Payload::LoadHistory(SessionHistory::Replace(messages)) =
        request(session, SessionCommand::LoadHistory)?
    else {
        return Err("expected replacement history".into());
    };
    json!(messages)
        .to_string()
        .contains(expected)
        .then_some(())
        .ok_or_else(|| format!("LoadHistory omitted {expected:?}"))
}

fn require_usage_response(session: &mut dyn SessionTransport) -> Result<(), String> {
    let Payload::LoadUsage(usage) = request(session, SessionCommand::LoadUsage)? else {
        return Err("expected usage response".into());
    };
    if usage.tokens.input == 0 || usage.tokens.output == 0 {
        return Err(format!("session usage is empty: {usage:?}"));
    }
    if usage.context_usage.is_none() {
        return Err("backend omitted context usage".into());
    }
    Ok(())
}

fn poll_until(
    session: &mut dyn SessionTransport,
    conversation: &mut ConversationState,
    lifecycle: &mut Lifecycle,
    responses: &mut HashMap<String, SessionResponse>,
    timeout: Duration,
    mut done: impl FnMut(&mut dyn SessionTransport, &Value, &ConversationState) -> Result<bool, String>,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let Some(item) = session.poll() else {
            thread::park_timeout(Duration::from_secs(1));
            continue;
        };
        match item {
            SessionEvent::Activity(event) => {
                lifecycle.observe(event.value());
                conversation.reduce(event.value());
                if done(session, event.value(), conversation)? {
                    return Ok(());
                }
            }
            SessionEvent::Interaction(request) => approve(session, request)?,
            SessionEvent::Response(response) => {
                if let Some(id) = response.id.clone() {
                    responses.insert(id, response);
                }
            }
            SessionEvent::Failure(error) => return Err(error),
            SessionEvent::Stderr(_) => {}
        }
    }
    Err(format!("timed out after {} seconds", timeout.as_secs()))
}

fn require_response(
    responses: &HashMap<String, SessionResponse>,
    id: &str,
    operation: SessionOperation,
) -> Result<(), String> {
    let response = responses
        .get(id)
        .ok_or_else(|| format!("missing response for {operation:?}"))?;
    if response.operation() == operation && response.result.is_ok() {
        Ok(())
    } else {
        Err(format!("invalid {operation:?} response: {response:?}"))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_harness_selector_is_strict() -> Result<(), String> {
        assert_eq!(select_harnesses(None)?, LIVE_HARNESSES);
        assert_eq!(
            known_backend_descriptors()
                .iter()
                .map(|descriptor| descriptor.id.as_str())
                .collect::<Vec<_>>(),
            LIVE_HARNESSES,
        );
        for harness in LIVE_HARNESSES {
            assert_eq!(select_harnesses(Some(harness))?, [harness]);
        }
        for selected in ["", "codex", "unknown"] {
            assert!(select_harnesses(Some(selected)).is_err());
        }
        Ok(())
    }

    #[test]
    fn empty_sessions_do_not_fail_compaction_conformance() {
        assert!(compaction_not_needed(
            "Nothing to compact (session too small)"
        ));
        assert!(!compaction_not_needed("provider unavailable"));
    }
}
