// Live-test diagnostics are consumed by the E2E runner.
#![allow(clippy::print_stderr)]
use crate::Backend;
use crate::{SessionHistory, SessionResponsePayload as Payload};
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
    conversation::{ConversationState, TranscriptKind},
};

use super::super::contract::{AgentBackendDescriptor, AgentCapabilities, CapabilitySupport};
use super::{
    delete_external_session, farcaster_mcp, known_backend_descriptors, load_external_history,
    main_session::external_session_locator, spawn_session,
};

pub const TURN_TIMEOUT: Duration = Duration::from_secs(180);
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
pub const LIVE_HARNESSES: [&str; 6] = [
    "pi",
    "codex-cli",
    "cursor-cli",
    "opencode",
    "claude",
    "antigravity-acp",
];
pub const TEST_IMAGE: &str = "iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAIAAAD8GO2jAAAAKklEQVR4nGP4EKBBU8QwasGoBaMWjFowasGoBaMWjFowasGoBaMWDBULACvxoEydbL2eAAAAAElFTkSuQmCC";

pub struct McpGuard;

impl McpGuard {
    pub fn disabled() -> Self {
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

pub fn select_harnesses(selected: Option<&str>) -> Result<Vec<&'static str>, String> {
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

pub fn descriptor(harness: Backend) -> Result<AgentBackendDescriptor, String> {
    known_backend_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.id == harness)
        .ok_or_else(|| format!("live harness {harness} has no backend descriptor"))
}

#[test]
#[ignore = "runs all six backends against live LLM accounts; consumes usage and retains sessions when deletion is unsupported"]
fn live_e2e_session_catalog_model_resume_move_delete() -> Result<(), String> {
    let _mcp = McpGuard::disabled();
    let selected = std::env::var("FARCASTER_E2E_HARNESS").ok();
    for harness in select_harnesses(selected.as_deref())? {
        let harness = harness.parse::<Backend>()?;
        let descriptor = descriptor(harness)?;
        exercise_live_harness(harness, &descriptor.capabilities)
            .map_err(|error| format!("{harness} live conformance failed: {error}"))?;
    }
    Ok(())
}

fn exercise_live_harness(harness: Backend, capabilities: &AgentCapabilities) -> Result<(), String> {
    let coverage = Coverage::from_capabilities(capabilities);
    let case_dir = support::e2e_case_dir()?;
    let project_guard = tempfile::tempdir_in(&case_dir)
        .map_err(|error| format!("create isolated live-test project: {error}"))?;
    fs::write(
        project_guard.path().join("AGENTS.md"),
        "# Live E2E fixture\n\nUse only files in this directory. Do not inspect or modify parent directories. Run only the exact tool command requested by the prompt.\n",
    )
    .map_err(|error| format!("write isolated project instructions: {error}"))?;
    let project = project_guard
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let locator_root = support::isolated_locator_root()?;
    let config = AgentLaunchConfig {
        program: PathBuf::from(harness.as_str()),
        prefix_args: Vec::new(),
        access_mode: support::live_access_mode_for_harness(harness)?,
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
            "E2E_LIMIT: {harness} live test session {} will remain because deletion is unsupported",
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
        if coverage.history && harness == Backend::Pi {
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
    // A temporary live project intentionally blocks catalog discovery and
    // therefore move coverage.  Do not let that expected limitation skip the
    // independent resume, history, persistence, and cleanup checks below.
    let move_outcome = if coverage.move_project {
        exercise_live_move(harness, &config, &project, &path, &marker, coverage)
    } else {
        Ok(())
    };
    let persistence_outcome = verify_persistence_and_cleanup(
        harness, &config, &launch, &path, &marker, coverage, &project,
    );
    match (move_outcome, persistence_outcome) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(move_error), Ok(())) => Err(move_error),
        (Ok(()), Err(error)) => Err(cleanup_error(error, Ok(()), harness, &path, coverage)),
        (Err(move_error), Err(error)) => Err(format!(
            "{}; persistence/cleanup: {}",
            move_error.replacen("E2E_BLOCKED: ", "move/catalog coverage blocked: ", 1),
            cleanup_error(error, Ok(()), harness, &path, coverage)
        )),
    }
}

fn exercise_live_move(
    harness: Backend,
    config: &AgentLaunchConfig,
    source: &Path,
    path: &Path,
    marker: &str,
    coverage: Coverage,
) -> Result<(), String> {
    if farcaster_projects::is_temporary_project(source) {
        return Err(format!(
            "E2E_BLOCKED: {} catalog discovery intentionally excludes the per-case temporary project {}; move/catalog coverage needs an explicit non-temporary project opt-in",
            harness,
            source.display()
        ));
    }
    let destination_guard =
        tempfile::tempdir_in(support::e2e_case_dir()?).map_err(|error| error.to_string())?;
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
            || (harness != Backend::Pi && stored.path != original.path)
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
            let history = super::load_session_history(harness, &current.path, project)?;
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
                    harness,
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
    harness: Backend,
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
                .ok_or_else(|| "E2E_BLOCKED: current model is unknown; refusing to change native defaults to an arbitrary catalog model".to_owned())?;
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
        if let Some(level) = state
            .thinking_level
            .as_ref()
            .filter(|level| levels.contains(level))
        {
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
    let marker = support::marker("legacy_normal");
    let steer_marker = support::marker("legacy_steer");
    let follow_up_marker = support::marker("legacy_follow_up");
    let final_marker = if coverage.follow_up {
        &follow_up_marker
    } else if coverage.steer {
        &steer_marker
    } else {
        &marker
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
        |session, event, conversation| {
            if event.get("type").and_then(Value::as_str) == Some("tool_execution_start")
                && coverage.steer
                && !steered
            {
                let id = session.send(SessionCommand::Prompt {
                    mode: PromptMode::Steer,
                    message: format!(
                        "Your final response must also include the exact token {steer_marker}."
                    ),
                    images: Vec::new(),
                })?;
                steer_id = Some(id);
                steered = true;
            }
            let settled = event.get("type").and_then(Value::as_str) == Some("agent_settled");
            if coverage.steer {
                Ok(steered && settled && conversation_contains(conversation, &steer_marker))
            } else {
                Ok(settled)
            }
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
            &steer_marker
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
            message: format!(
                "Use the shell tool to run `sleep 2`; then reply with {}.",
                support::marker("legacy_queue_base")
            ),
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
                        message: format!(
                            "After the current turn, reply with the exact token {follow_up_marker}."
                        ),
                        images: Vec::new(),
                    })?;
                    follow_up_id = Some(id);
                    follow_up_sent = true;
                }
                Ok(conversation_contains(conversation, &follow_up_marker)
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
    Ok(final_marker.to_owned())
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
            Some(SessionEvent::Interaction(request)) => approve(session, request, &[])?,
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
            Some(SessionEvent::Interaction(request)) => approve(session, request, &[])?,
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

fn cleanup_failed_fixture(harness: Backend, path: &Path, coverage: Coverage) -> Result<(), String> {
    if harness == Backend::Pi {
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
    harness: Backend,
    config: &AgentLaunchConfig,
    launch: &impl Fn(SessionStart, Option<String>) -> SessionLaunch,
    path: &Path,
    marker: &str,
    coverage: Coverage,
    project: &Path,
) -> Result<(), String> {
    if harness == Backend::Pi {
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
        let history = load_external_history(path, project)
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
            Some(SessionEvent::Interaction(request)) => approve(session, request, &[])?,
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
    let Payload::LoadHistory(SessionHistory::Replace { messages, .. }) =
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
            SessionEvent::Interaction(request) => approve(session, request, &[])?,
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

fn approve(
    session: &mut dyn SessionTransport,
    request: ExtensionUiRequest,
    allowed_commands: &[String],
) -> Result<(), String> {
    match request {
        // These mutate only harness UI state.  They have no dialog ID under
        // the production contract, so there is no response to send and no
        // permission to grant.
        ExtensionUiRequest::Notify { .. }
        | ExtensionUiRequest::SetStatus { .. }
        | ExtensionUiRequest::SetWidget { .. }
        | ExtensionUiRequest::SetTitle { .. }
        | ExtensionUiRequest::SetEditorText { .. } => Ok(()),
        request => {
            let response = support::bounded_command_permission(&request, allowed_commands)?;
            session.respond(response)
        }
    }
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

/// Test-only support for ignored tests that run the installed harness against a
/// real account.  It deliberately speaks only through `spawn_session` and the
/// normalized transport; fixture processes and injected activities belong in
/// adapter tests, not here.
pub mod support {
    use super::*;

    pub use super::{McpGuard, TEST_IMAGE, TURN_TIMEOUT};

    pub const EVENT_POLL: Duration = Duration::from_millis(20);

    #[derive(Clone, Debug)]
    pub struct Submission {
        pub id: String,
        pub mode: PromptMode,
        pub text: String,
        /// `marker` is the exact caller-provided text.  Live tests use an
        /// unguessable marker in it, rather than a process ID or fixed token.
        pub marker: String,
        pub images: Vec<PromptImage>,
    }

    /// The strongest receipt evidence a harness actually exposes.  Native
    /// history is enough for one unique functional marker, never for matching
    /// equal text or delayed image receipts.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum PromptObservation {
        CorrelatedDelivery,
        NativeHistoryOnly,
    }

    /// The exact provider/model pair accepted by a short-lived real harness
    /// session.  Child E2E profiles use this instead of guessing from the
    /// first entry in a possibly stale configuration catalog.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct LiveWorkerModel {
        pub provider: String,
        pub model: String,
    }

    #[derive(Clone, Debug)]
    pub struct TraceEvent {
        pub elapsed: Duration,
        pub value: Value,
    }

    #[derive(Clone, Debug)]
    pub struct TurnGate {
        file_name: String,
        script_path: PathBuf,
        pid_path: PathBuf,
        started_path: PathBuf,
        release_path: PathBuf,
        timeout_path: PathBuf,
        pub marker: String,
    }

    impl TurnGate {
        pub fn new(project: &Path, label: &str) -> Result<Self, String> {
            let suffix = uuid::Uuid::new_v4().simple().to_string();
            let file_name = format!("farcaster-live-gate-{suffix}");
            let script_path = project.join(format!("{file_name}.sh"));
            let pid_path = project.join(format!("{file_name}.pid"));
            let started_path = project.join(format!("{file_name}.started"));
            let release_path = project.join(&file_name);
            let timeout_path = project.join(format!("{file_name}.timed-out"));
            if script_path.exists()
                || pid_path.exists()
                || started_path.exists()
                || release_path.exists()
                || timeout_path.exists()
            {
                return Err(format!(
                    "live test gate unexpectedly already exists: {}, {}, {}, {}, or {}",
                    script_path.display(),
                    pid_path.display(),
                    started_path.display(),
                    release_path.display(),
                    timeout_path.display()
                ));
            }
            let gate = Self {
                script_path,
                pid_path,
                started_path,
                release_path,
                timeout_path,
                file_name,
                marker: format!("FARCASTER_GATE_{}_{}", label.to_ascii_uppercase(), suffix),
            };
            fs::write(&gate.script_path, gate.script()).map_err(|error| {
                format!(
                    "write deterministic live tool gate {}: {error}",
                    gate.script_path.display()
                )
            })?;
            Ok(gate)
        }

        pub fn file_name(&self) -> &str {
            &self.file_name
        }

        /// The only shell command a live E2E permission responder may allow.
        /// It runs the owned, project-local gate without arguments or shell
        /// composition.
        pub fn shell_command(&self) -> String {
            format!("sh ./{}", self.script_file_name())
        }

        pub fn prompt(&self) -> String {
            format!(
                "Use the shell tool to run exactly `{}`. Do not edit the script or any gate files. Do not answer before it exits. Then reply with the exact token {}.",
                self.shell_command(),
                self.marker
            )
        }

        /// UI tests can release a real tool process without owning the
        /// transport that started it.
        pub fn release(&self) -> Result<(), String> {
            self.assert_still_closed()?;
            fs::write(&self.release_path, format!("{}\n", self.marker))
                .map_err(|error| format!("release live tool gate {}: {error}", self.file_name))
        }

        /// Control tests call this before release.  It rules out the tool's
        /// bounded timeout as the reason a turn settled or a handoff started.
        pub fn assert_still_closed(&self) -> Result<(), String> {
            if !self.script_path.is_file() {
                return Err(format!(
                    "live tool gate {} script disappeared before the control assertion",
                    self.file_name
                ));
            }
            if self.timeout_path.exists() {
                return Err(format!(
                    "live tool gate {} timed out before the control assertion",
                    self.file_name
                ));
            }
            if self.release_path.exists() {
                return Err(format!(
                    "live tool gate {} was released before the control assertion",
                    self.file_name
                ));
            }
            Ok(())
        }

        pub fn has_started(&self) -> bool {
            self.started_path.is_file()
        }

        pub fn assert_started(&self) -> Result<(), String> {
            self.assert_still_closed()?;
            if self.has_started() {
                Ok(())
            } else {
                Err(format!(
                    "live tool gate {} did not write its shell-start witness",
                    self.file_name
                ))
            }
        }

        /// Verifies that the PID emitted by the live shell gate is observable
        /// before a control command.  This calibrates `kill -0` against a
        /// process we own, so a later failure is meaningful exit evidence.
        pub fn assert_process_alive(&self) -> Result<(), String> {
            self.assert_started()?;
            let pid = self.pid()?;
            if gate_process_alive(pid)? {
                Ok(())
            } else {
                Err(format!(
                    "live tool gate {} wrote PID {pid}, but /bin/kill -0 could not observe it before control",
                    self.file_name
                ))
            }
        }

        /// After an acknowledged Abort, waits a bounded time for exactly the
        /// shell process that opened this gate to exit.  It sends no signal;
        /// `kill -0` only probes liveness.  A release or natural timeout makes
        /// this fail rather than masking a no-op Abort.
        pub fn assert_process_exited_after_abort(&self) -> Result<(), String> {
            self.assert_still_closed()?;
            self.assert_started()?;
            let pid = self.pid()?;
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if !gate_process_alive(pid)? {
                    return Ok(());
                }
                thread::park_timeout(EVENT_POLL);
                self.assert_still_closed()?;
            }
            Err(format!(
                "live tool gate {} PID {pid} remained alive for 10 seconds after Abort",
                self.file_name
            ))
        }

        fn pid(&self) -> Result<u32, String> {
            let pid = fs::read_to_string(&self.pid_path)
                .map_err(|error| {
                    format!(
                        "read live tool gate PID witness {}: {error}",
                        self.pid_path.display()
                    )
                })?
                .trim()
                .parse::<u32>()
                .map_err(|error| {
                    format!(
                        "parse live tool gate PID witness {}: {error}",
                        self.pid_path.display()
                    )
                })?;
            if pid == 0 {
                return Err(format!(
                    "live tool gate PID witness {} must not be zero",
                    self.pid_path.display()
                ));
            }
            Ok(pid)
        }

        fn script(&self) -> String {
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$$\" > '{}'\nprintf started > '{}'\nfor i in $(seq 1 4800); do\n  if [ ! -f './{}' ]; then\n    exit 125\n  fi\n  if [ -s '{}' ]; then\n    cat '{}'\n    exit 0\n  fi\n  sleep 0.1\ndone\nprintf timed-out > '{}'\nexit 124\n",
                self.pid_file_name(),
                self.started_file_name(),
                self.script_file_name(),
                self.file_name,
                self.file_name,
                self.timeout_file_name(),
            )
        }

        fn script_file_name(&self) -> &str {
            self.script_path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("turn gate script fixture name is valid UTF-8")
        }

        fn started_file_name(&self) -> &str {
            self.started_path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("turn gate start fixture name is valid UTF-8")
        }

        fn pid_file_name(&self) -> &str {
            self.pid_path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("turn gate PID fixture name is valid UTF-8")
        }

        fn timeout_file_name(&self) -> &str {
            self.timeout_path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("turn gate timeout fixture name is valid UTF-8")
        }
    }

    /// Returns a one-shot reply only when an installed client asks to run one
    /// of the exact project-local commands registered by this test.
    ///
    /// This recognizes only the three observed permission forms.  In
    /// particular, it never chooses an "always" option and never accepts a
    /// title which merely contains an allowed command.
    pub fn bounded_command_permission(
        request: &ExtensionUiRequest,
        allowed_commands: &[String],
    ) -> Result<ExtensionUiResponse, String> {
        let ExtensionUiRequest::Select {
            id, title, options, ..
        } = request
        else {
            return Err(format!(
                "E2E_BLOCKED: live harness requested unmanaged interaction {request:?}; refusing to grant permission outside the fixture scope"
            ));
        };
        let exact_command =
            |command: &str| allowed_commands.iter().any(|allowed| command == allowed);
        let options_are = |expected: &[&str]| {
            options.len() == expected.len()
                && options
                    .iter()
                    .map(String::as_str)
                    .eq(expected.iter().copied())
        };
        let response = if options_are(&["Deny", "Allow"]) {
            let payload = title.strip_prefix("Allow Bash?\n").ok_or_else(|| {
                format!(
                    "E2E_BLOCKED: refusing non-gate Bash selection while waiting for a registered command: {title:?}"
                )
            })?;
            let payload: Value = serde_json::from_str(payload).map_err(|error| {
                format!("E2E_BLOCKED: refusing malformed Bash permission payload: {error}")
            })?;
            let command = payload
                .as_object()
                .and_then(|object| object.get("command"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    "E2E_BLOCKED: refusing Bash permission without an exact command".to_owned()
                })?;
            if !exact_command(command) {
                return Err(format!(
                    "E2E_BLOCKED: refusing Bash command outside the registered project-local fixture: {command:?}"
                ));
            }
            "Allow"
        } else if options_are(&["Allow once", "Allow always", "Reject"]) {
            let cursor_title_matches = exact_command(title)
                || allowed_commands
                    .iter()
                    .any(|command| title == &format!("`{command}`"));
            if !cursor_title_matches {
                return Err(format!(
                    "E2E_BLOCKED: refusing Cursor command outside the registered project-local fixture: {title:?}"
                ));
            }
            "Allow once"
        } else if options_are(&["Allow Always (risky)", "Allow", "Deny"]) {
            if !exact_command(title) {
                return Err(format!(
                    "E2E_BLOCKED: refusing ACP command outside the registered project-local fixture: {title:?}"
                ));
            }
            "Allow"
        } else {
            return Err(format!(
                "E2E_BLOCKED: refusing command permission with unexpected choices: {options:?}"
            ));
        };
        Ok(ExtensionUiResponse::Value {
            id: id.clone(),
            value: response.into(),
        })
    }

    /// Returns a reply only for the exact Bash permission required by one of
    /// this process's registered project-local gates.  Live tests never grant
    /// a general shell permission, even though the fixture itself is safe.
    pub fn bounded_gate_permission(
        request: &ExtensionUiRequest,
        gates: &[TurnGate],
    ) -> Result<ExtensionUiResponse, String> {
        let commands = gates
            .iter()
            .map(TurnGate::shell_command)
            .collect::<Vec<_>>();
        bounded_command_permission(request, &commands)
    }

    fn gate_process_alive(pid: u32) -> Result<bool, String> {
        use std::process::{Command, Stdio};

        let pid_text = pid.to_string();
        let output = Command::new("/bin/kill")
            .args(["-0", pid_text.as_str()])
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("run /bin/kill -0 for live gate PID {pid}: {error}"))?;
        if output.status.success() {
            return Ok(true);
        }
        let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
        if stderr.contains("no such process") {
            return Ok(false);
        }
        Err(format!(
            "/bin/kill -0 for live gate PID {pid} did not prove exit (status={}, stderr={stderr:?})",
            output.status
        ))
    }

    /// An isolated project plus a single real installed harness session.
    ///
    /// `FARCASTER_DATA_DIR` is set by `scripts/e2e.sh`.  We reject an absent
    /// setting here so a developer cannot accidentally point the live suite at
    /// their normal Farcaster database.
    pub struct LiveSession {
        harness: Backend,
        capabilities: AgentCapabilities,
        project_guard: tempfile::TempDir,
        _locator_guard: tempfile::TempDir,
        config: AgentLaunchConfig,
        transport: Box<dyn SessionTransport>,
        path: PathBuf,
        conversation: ConversationState,
        responses: HashMap<String, SessionResponse>,
        activities: Vec<TraceEvent>,
        stderr: Vec<String>,
        gates: Vec<TurnGate>,
        fixture_commands: Vec<String>,
        submitted_prompt: bool,
        started_at: Instant,
        program_version: String,
        model_identity: Option<String>,
        _mcp: McpGuard,
    }

    impl LiveSession {
        pub fn start(harness: Backend) -> Result<Self, String> {
            let case_dir = e2e_case_dir()?;
            let descriptor = descriptor(harness)?;
            let project_guard = tempfile::tempdir_in(&case_dir)
                .map_err(|error| format!("create isolated live-test project: {error}"))?;
            fs::write(
                project_guard.path().join("AGENTS.md"),
                "# Live E2E fixture\n\nUse only files in this directory. Do not inspect or modify parent directories. Run only the exact tool command requested by the prompt.\n",
            )
            .map_err(|error| format!("write isolated project instructions: {error}"))?;
            let locator_guard = tempfile::tempdir_in(project_guard.path())
                .map_err(|error| format!("create isolated live locator root: {error}"))?;
            let project = project_guard
                .path()
                .canonicalize()
                .map_err(|error| format!("canonicalize live project: {error}"))?;
            let config = AgentLaunchConfig {
                program: PathBuf::from(harness.as_str()),
                prefix_args: Vec::new(),
                access_mode: live_access_mode_for_harness(harness)?,
                app_proxy: None,
                session_locator_root: Some(locator_guard.path().into()),
            };
            let resolved_config = super::super::launch_configuration(&config, harness)?;
            let program_version = program_version(&resolved_config.program);
            let _mcp = McpGuard::disabled();
            let transport = spawn_session(
                &config,
                SessionLaunch {
                    harness,
                    session_id: None,
                    project,
                    start: SessionStart::New,
                    wake: Some(thread::current()),
                },
            )?;
            let mut live = Self {
                harness,
                capabilities: descriptor.capabilities,
                project_guard,
                _locator_guard: locator_guard,
                config,
                transport,
                path: PathBuf::new(),
                conversation: ConversationState::default(),
                responses: HashMap::new(),
                activities: Vec::new(),
                stderr: Vec::new(),
                gates: Vec::new(),
                fixture_commands: Vec::new(),
                submitted_prompt: false,
                started_at: Instant::now(),
                program_version,
                model_identity: None,
                _mcp,
            };
            let state = live.load_state()?;
            live.path = state
                .session_file
                .map(PathBuf::from)
                .ok_or_else(|| "live session state omitted its locator path".to_owned())?;
            live.model_identity = state.model.map(|model| {
                format!(
                    "{}/{}",
                    model.provider,
                    model.resolved_model.unwrap_or(model.id)
                )
            });
            eprintln!(
                "live E2E harness={} version={} model={}",
                live.harness,
                live.program_version,
                live.model_identity.as_deref().unwrap_or("unreported")
            );
            Ok(live)
        }

        pub fn harness(&self) -> &str {
            self.harness.as_str()
        }

        pub fn capabilities(&self) -> &AgentCapabilities {
            &self.capabilities
        }

        pub fn project(&self) -> &Path {
            self.project_guard.path()
        }

        pub fn session_path(&mut self) -> Result<PathBuf, String> {
            let Payload::LoadState(state) = self.request(SessionCommand::LoadState)? else {
                return Err("expected state response".into());
            };
            state
                .session_file
                .map(PathBuf::from)
                .ok_or_else(|| "live session state omitted its locator path".to_owned())
        }

        pub fn conversation(&self) -> &ConversationState {
            &self.conversation
        }

        pub fn activities(&self) -> &[TraceEvent] {
            &self.activities
        }

        pub fn require_available(
            &self,
            feature: &str,
            support: &CapabilitySupport,
        ) -> Result<(), String> {
            if *support == CapabilitySupport::Available {
                Ok(())
            } else {
                Err(format!(
                    "E2E_BLOCKED: {} declares {feature} unsupported; live E2E may not mark this feature passed",
                    self.harness
                ))
            }
        }

        pub fn configure_steering(&mut self) -> Result<(), String> {
            self.request(SessionCommand::ConfigureSteering)?;
            Ok(())
        }

        pub fn submit(
            &mut self,
            mode: PromptMode,
            text: impl Into<String>,
            images: Vec<PromptImage>,
        ) -> Result<Submission, String> {
            let text = text.into();
            let id = self.transport.send(SessionCommand::Prompt {
                mode,
                message: text.clone(),
                images: images.clone(),
            })?;
            self.submitted_prompt = true;
            Ok(Submission {
                id,
                mode,
                marker: text.clone(),
                text,
                images,
            })
        }

        pub fn tracks_prompt_delivery(&self, mode: PromptMode) -> bool {
            self.transport.tracks_prompt_delivery(mode)
        }

        pub fn require_prompt_delivery_tracking(&self, mode: PromptMode) -> Result<(), String> {
            if self.tracks_prompt_delivery(mode) {
                Ok(())
            } else {
                Err(format!(
                    "E2E_BLOCKED: {} does not expose exact delivery IDs for {mode:?}; refusing text/FIFO inference",
                    self.harness
                ))
            }
        }

        pub fn require_functional_prompt_observation(
            &self,
            mode: PromptMode,
        ) -> Result<PromptObservation, String> {
            if self.tracks_prompt_delivery(mode) {
                return Ok(PromptObservation::CorrelatedDelivery);
            }
            self.require_available("native history", &self.capabilities.sessions.history)?;
            eprintln!(
                "E2E_LIMIT: {} has no exact delivery IDs for {mode:?}; using unique-marker native history only",
                self.harness
            );
            Ok(PromptObservation::NativeHistoryOnly)
        }

        pub fn apply_steering(&mut self) -> Result<String, String> {
            self.transport.send(SessionCommand::ApplySteering)
        }

        pub fn abort(&mut self) -> Result<String, String> {
            self.transport.send(SessionCommand::Abort)
        }

        /// Registers one literal command for the live fixture's narrow
        /// one-shot permission responder.  Callers must use the returned
        /// value verbatim in their prompt; the responder never extracts a
        /// command from model or backend output.
        pub fn register_fixture_command(
            &mut self,
            command: impl Into<String>,
        ) -> Result<String, String> {
            let command = command.into();
            if command.is_empty()
                || command.trim() != command.as_str()
                || command.chars().any(|character| {
                    matches!(
                        character,
                        '\n' | '\r' | ';' | '|' | '&' | '`' | '$' | '(' | ')' | '<' | '>'
                    )
                })
            {
                return Err(format!(
                    "refusing unsafe live fixture command registration: {command:?}"
                ));
            }
            self.fixture_commands.push(command.clone());
            Ok(command)
        }

        /// Ask the real model to hold an actual shell invocation.  Tests wait
        /// for that observed tool event before writing the release file.
        pub fn start_gated_turn(&mut self, label: &str) -> Result<TurnGate, String> {
            self.require_available(
                "tool activity",
                &self.capabilities.observation.tool_activity,
            )?;
            let gate = TurnGate::new(self.project(), label)?;
            // Register before the real prompt leaves this process.  Claude
            // can ask its exact Bash permission before any tool activity.
            self.gates.push(gate.clone());
            self.submit(PromptMode::Normal, gate.prompt(), Vec::new())?;
            self.wait_for_tool_start(gate.file_name(), TURN_TIMEOUT)?;
            self.wait_for_gate_started(&gate, TURN_TIMEOUT)?;
            gate.assert_process_alive()?;
            Ok(gate)
        }

        pub fn release_gate(&mut self, gate: &TurnGate) -> Result<(), String> {
            gate.release()
        }

        pub fn activity_cursor(&self) -> usize {
            self.activities.len()
        }

        pub fn wait_for_tool_start(
            &mut self,
            gate_file: &str,
            timeout: Duration,
        ) -> Result<Value, String> {
            self.wait_for_activity(timeout, |event| {
                event.get("type").and_then(Value::as_str) == Some("tool_execution_start")
                    && event.to_string().contains(gate_file)
            })
            .map_err(|error| format!("{error}; trace={}", self.trace_summary()))
        }

        /// Finds the one real shell tool invocation made by this gate.  The
        /// generated gate filename is unique per case, so this never matches
        /// another turn by text order.
        pub fn gate_tool_call_id(&self, gate: &TurnGate) -> Result<String, String> {
            let matching = self
                .activities
                .iter()
                .filter(|event| {
                    event.value.get("type").and_then(Value::as_str) == Some("tool_execution_start")
                        && event.value.to_string().contains(gate.file_name())
                })
                .collect::<Vec<_>>();
            if matching.len() != 1 {
                return Err(format!(
                    "expected exactly one live gate tool start for {}, found {}; trace={}",
                    gate.file_name(),
                    matching.len(),
                    self.trace_summary()
                ));
            }
            matching[0]
                .value
                .get("toolCallId")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    format!(
                        "live gate tool start omitted toolCallId: {}",
                        matching[0].value
                    )
                })
        }

        /// Waits for the actual gated shell invocation to finish after a
        /// control command.  Harnesses such as OpenCode can keep the outer
        /// turn alive after ApplySteering, so an unrelated `agent_start` is
        /// not evidence that the control took effect.
        pub fn wait_for_gate_tool_end_after(
            &mut self,
            cursor: usize,
            gate: &TurnGate,
            timeout: Duration,
        ) -> Result<Value, String> {
            let tool_call_id = self.gate_tool_call_id(gate)?;
            self.wait_for_activity_after(cursor, timeout, |event| {
                event.get("type").and_then(Value::as_str) == Some("tool_execution_end")
                    && event.get("toolCallId").and_then(Value::as_str)
                        == Some(tool_call_id.as_str())
            })
        }

        /// Proves the first-Escape handoff reached a real turn boundary without
        /// assuming every harness starts a replacement turn.  Some continue
        /// the outer turn and finish the original shell tool; others settle it
        /// and begin a new turn.  The tool path uses the exact observed call
        /// ID, while the replacement path requires `agent_start` after—not
        /// merely near—the matching settlement.
        pub fn wait_for_apply_handoff_after(
            &mut self,
            cursor: usize,
            gate: &TurnGate,
            timeout: Duration,
        ) -> Result<Value, String> {
            let tool_call_id = self.gate_tool_call_id(gate)?;
            let deadline = Instant::now() + timeout;
            let mut inspected = cursor;
            let mut settled = false;

            while Instant::now() < deadline {
                while let Some(event) = self.activities.get(inspected) {
                    inspected += 1;
                    let kind = event.value.get("type").and_then(Value::as_str);
                    if kind == Some("tool_execution_end")
                        && event.value.get("toolCallId").and_then(Value::as_str)
                            == Some(tool_call_id.as_str())
                    {
                        return Ok(event.value.clone());
                    }
                    if kind == Some("agent_settled") {
                        settled = true;
                        continue;
                    }
                    if settled && kind == Some("agent_start") {
                        return Ok(event.value.clone());
                    }
                }
                self.pump_one()?;
            }

            Err(format!(
                "timed out after {} seconds waiting for ApplySteering to finish gate tool {} or start after settlement; trace={}",
                timeout.as_secs(),
                tool_call_id,
                self.trace_summary()
            ))
        }

        /// A cancelled gate may report an end event as part of cancellation,
        /// so its end alone cannot prove later execution.  A new start of the
        /// uniquely named gate after settlement does prove that the stopped
        /// shell work restarted.  Keep this tied to the gate filename rather
        /// than any assistant text, which models may echo or reason about.
        pub fn assert_no_gate_tool_start_after(
            &self,
            cursor: usize,
            gate: &TurnGate,
        ) -> Result<(), String> {
            if let Some(event) = self.activities.iter().skip(cursor).find(|event| {
                event.value.get("type").and_then(Value::as_str) == Some("tool_execution_start")
                    && event.value.to_string().contains(gate.file_name())
            }) {
                return Err(format!(
                    "cancelled gate {} started real shell work after settlement: +{}ms {}; trace={}",
                    gate.file_name(),
                    event.elapsed.as_millis(),
                    event.value,
                    self.trace_summary()
                ));
            }
            Ok(())
        }

        pub fn wait_for_gate_started(
            &mut self,
            gate: &TurnGate,
            timeout: Duration,
        ) -> Result<(), String> {
            let deadline = Instant::now() + timeout;
            while Instant::now() < deadline {
                if gate.has_started() {
                    return gate.assert_started();
                }
                self.pump_one()?;
            }
            Err(format!(
                "timed out after {} seconds waiting for shell-start witness for {}",
                timeout.as_secs(),
                gate.file_name()
            ))
        }

        pub fn wait_for_activity(
            &mut self,
            timeout: Duration,
            predicate: impl Fn(&Value) -> bool,
        ) -> Result<Value, String> {
            if let Some(event) = self
                .activities
                .iter()
                .map(|event| &event.value)
                .find(|event| predicate(event))
            {
                return Ok(event.clone());
            }
            let deadline = Instant::now() + timeout;
            while Instant::now() < deadline {
                self.pump_one()?;
                if let Some(event) = self
                    .activities
                    .iter()
                    .map(|event| &event.value)
                    .rfind(|event| predicate(event))
                {
                    return Ok(event.clone());
                }
            }
            Err(format!(
                "timed out after {} seconds waiting for activity",
                timeout.as_secs()
            ))
        }

        pub fn wait_for_activity_after(
            &mut self,
            cursor: usize,
            timeout: Duration,
            predicate: impl Fn(&Value) -> bool,
        ) -> Result<Value, String> {
            if let Some(event) = self
                .activities
                .iter()
                .skip(cursor)
                .map(|event| &event.value)
                .find(|event| predicate(event))
            {
                return Ok(event.clone());
            }
            let deadline = Instant::now() + timeout;
            while Instant::now() < deadline {
                self.pump_one()?;
                if let Some(event) = self
                    .activities
                    .iter()
                    .skip(cursor)
                    .map(|event| &event.value)
                    .rfind(|event| predicate(event))
                {
                    return Ok(event.clone());
                }
            }
            Err(format!(
                "timed out after {} seconds waiting for activity after trace position {cursor}",
                timeout.as_secs()
            ))
        }

        pub fn wait_for_response(
            &mut self,
            id: &str,
            timeout: Duration,
        ) -> Result<SessionResponse, String> {
            if let Some(response) = self.responses.get(id) {
                return Ok(response.clone());
            }
            let deadline = Instant::now() + timeout;
            while Instant::now() < deadline {
                self.pump_one()?;
                if let Some(response) = self.responses.get(id) {
                    return Ok(response.clone());
                }
            }
            Err(format!(
                "timed out after {} seconds waiting for response {id}; trace={}",
                timeout.as_secs(),
                self.trace_summary()
            ))
        }

        pub fn wait_for_delivery_after(
            &mut self,
            cursor: usize,
            submission_id: &str,
            timeout: Duration,
        ) -> Result<Value, String> {
            self.wait_for_activity_after(cursor, timeout, |event| {
                event.get("type").and_then(Value::as_str) == Some("prompt_delivery")
                    && event.get("submissionId").and_then(Value::as_str) == Some(submission_id)
                    && event.get("status").and_then(Value::as_str) == Some("delivered")
            })
        }

        pub fn wait_for_functional_observation_after(
            &mut self,
            cursor: usize,
            submission: &Submission,
            marker: &str,
            timeout: Duration,
        ) -> Result<PromptObservation, String> {
            match self.require_functional_prompt_observation(submission.mode)? {
                PromptObservation::CorrelatedDelivery => {
                    self.wait_for_delivery_after(cursor, &submission.id, timeout)?;
                    Ok(PromptObservation::CorrelatedDelivery)
                }
                PromptObservation::NativeHistoryOnly => {
                    self.wait_for_native_user_marker(marker, &submission.images, timeout)?;
                    Ok(PromptObservation::NativeHistoryOnly)
                }
            }
        }

        pub fn wait_for_native_idle(&mut self, timeout: Duration) -> Result<(), String> {
            let deadline = Instant::now() + timeout;
            while Instant::now() < deadline {
                let state = self.load_state()?;
                if !state.is_streaming && !state.is_compacting && state.pending_message_count == 0 {
                    return Ok(());
                }
                self.pump_one()?;
                thread::park_timeout(Duration::from_millis(100));
            }
            Err(format!(
                "native session did not become idle before the later Normal prompt; trace={}",
                self.trace_summary()
            ))
        }

        pub fn wait_for_settled_after(
            &mut self,
            cursor: usize,
            timeout: Duration,
        ) -> Result<(), String> {
            self.wait_for_activity_after(cursor, timeout, |event| {
                event.get("type").and_then(Value::as_str) == Some("agent_settled")
            })?;
            Ok(())
        }

        pub fn wait_for_assistant_text(
            &mut self,
            expected: &str,
            timeout: Duration,
        ) -> Result<(), String> {
            let deadline = Instant::now() + timeout;
            while Instant::now() < deadline {
                if conversation_contains(&self.conversation, expected) {
                    return Ok(());
                }
                self.pump_one()?;
            }
            require_assistant_text(&self.conversation, expected)
                .map_err(|error| format!("{error}; trace={}", self.trace_summary()))
        }

        pub fn assert_no_delivery(&self, submission_id: &str) -> Result<(), String> {
            if self.activities.iter().any(|event| {
                event.value.get("type").and_then(Value::as_str) == Some("prompt_delivery")
                    && event.value.get("submissionId").and_then(Value::as_str)
                        == Some(submission_id)
                    && event.value.get("status").and_then(Value::as_str) == Some("delivered")
            }) {
                Err(format!(
                    "submission {submission_id} delivered before the test released its gate; trace={}",
                    self.trace_summary()
                ))
            } else {
                Ok(())
            }
        }

        pub fn assert_no_delivery_before(
            &self,
            cursor: usize,
            submission_id: &str,
        ) -> Result<(), String> {
            if self.activities.iter().skip(cursor).any(|event| {
                event.value.get("type").and_then(Value::as_str) == Some("prompt_delivery")
                    && event.value.get("submissionId").and_then(Value::as_str)
                        == Some(submission_id)
                    && event.value.get("status").and_then(Value::as_str) == Some("delivered")
            }) {
                Err(format!(
                    "submission {submission_id} delivered at or after forbidden event boundary {cursor}; trace={}",
                    self.trace_summary()
                ))
            } else {
                Ok(())
            }
        }

        pub fn assert_transcript_user_once(
            &self,
            marker: &str,
            expected_images: usize,
        ) -> Result<(), String> {
            let users = self
                .conversation
                .items
                .iter()
                .filter(|item| {
                    item.kind == TranscriptKind::User && item.complete_text().contains(marker)
                })
                .collect::<Vec<_>>();
            if users.len() != 1 {
                return Err(format!(
                    "expected exactly one user transcript row containing {marker:?}, found {}; transcript={}",
                    users.len(),
                    self.transcript_summary()
                ));
            }
            if users[0].images.len() != expected_images {
                return Err(format!(
                    "user transcript row {marker:?} has {} images, expected {expected_images}",
                    users[0].images.len()
                ));
            }
            Ok(())
        }

        pub fn assert_submission_once(&mut self, submission: &Submission) -> Result<(), String> {
            self.assert_transcript_user_once(&submission.marker, submission.images.len())?;
            self.assert_native_user_once(&submission.marker, &submission.images)
        }

        pub fn assert_functional_submission_once(
            &mut self,
            submission: &Submission,
            marker: &str,
        ) -> Result<(), String> {
            match self.require_functional_prompt_observation(submission.mode)? {
                PromptObservation::CorrelatedDelivery => self.assert_submission_once(submission),
                PromptObservation::NativeHistoryOnly => {
                    self.assert_transcript_user_once(marker, submission.images.len())?;
                    self.assert_native_user_once(marker, &submission.images)
                }
            }
        }

        /// Equal text cannot establish identity.  This assertion requires the
        /// native history and normalized delivery events to retain each real
        /// submission ID; a backend that cannot do so reports a blocked E2E
        /// case instead of guessing with text order.
        pub fn assert_duplicate_submissions_once(
            &mut self,
            first: &Submission,
            second: &Submission,
        ) -> Result<(), String> {
            if first.text != second.text || first.id == second.id {
                return Err(
                    "duplicate-submission assertion requires equal text and distinct IDs".into(),
                );
            }
            for submission in [first, second] {
                self.require_prompt_delivery_tracking(submission.mode)?;
                let delivered = self
                    .activities
                    .iter()
                    .filter(|event| {
                        event.value.get("type").and_then(Value::as_str) == Some("prompt_delivery")
                            && event.value.get("submissionId").and_then(Value::as_str)
                                == Some(submission.id.as_str())
                            && event.value.get("status").and_then(Value::as_str)
                                == Some("delivered")
                    })
                    .collect::<Vec<_>>();
                if delivered.len() != 1 {
                    return Err(format!(
                        "expected one correlated delivery for {}, found {}; trace={}",
                        submission.id,
                        delivered.len(),
                        self.trace_summary()
                    ));
                }
                require_event_images(&delivered[0].value, &submission.images)?;
            }
            let rows = self
                .conversation
                .items
                .iter()
                .filter(|item| {
                    item.kind == TranscriptKind::User && item.complete_text().contains(&first.text)
                })
                .collect::<Vec<_>>();
            if rows.len() != 2 || rows.iter().any(|item| item.images.len() != 1) {
                return Err(format!(
                    "equal-text submissions did not remain two one-image transcript rows: {}",
                    self.transcript_summary()
                ));
            }
            let history = self.history()?;
            for submission in [first, second] {
                let native = history
                    .iter()
                    .filter(|message| {
                        history_submission_id(message) == Some(submission.id.as_str())
                            && history_user_contains(message, &submission.text)
                    })
                    .collect::<Vec<_>>();
                if native.len() != 1 {
                    return Err(format!(
                        "E2E_BLOCKED: {} native history cannot prove exact record for submission {} (found {}); refusing equal-text identity inference",
                        self.harness,
                        submission.id,
                        native.len()
                    ));
                }
                require_wire_images(native[0], &submission.images)?;
            }
            Ok(())
        }

        pub fn assert_native_user_once(
            &mut self,
            marker: &str,
            expected_images: &[PromptImage],
        ) -> Result<(), String> {
            let history = self.history()?;
            let expected_images = expected_images
                .iter()
                .map(|image| {
                    let data = if image.data.is_empty() {
                        use base64::Engine as _;
                        base64::engine::general_purpose::STANDARD.encode(image.bytes()?)
                    } else {
                        image.data.clone()
                    };
                    Ok::<_, String>((data, image.mime_type.clone()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let matching = history
                .iter()
                .filter(|message| {
                    history_user_contains(message, marker)
                        && expected_images.iter().all(|(data, mime_type)| {
                            let wire = message.to_string();
                            wire.contains(data.as_str()) && wire.contains(mime_type.as_str())
                        })
                })
                .collect::<Vec<_>>();
            if matching.len() != 1 {
                return Err(format!(
                    "expected exactly one native user record containing {marker:?}, found {}; history={history:?}",
                    matching.len()
                ));
            }
            for (data, mime_type) in expected_images {
                let wire = matching[0].to_string();
                if !wire.contains(data.as_str()) || !wire.contains(mime_type.as_str()) {
                    return Err(format!(
                        "native user record for {marker:?} omitted exact image bytes or MIME {mime_type}: {}",
                        matching[0]
                    ));
                }
            }
            Ok(())
        }

        fn wait_for_native_user_marker(
            &mut self,
            marker: &str,
            images: &[PromptImage],
            timeout: Duration,
        ) -> Result<(), String> {
            let deadline = Instant::now() + timeout;
            let mut last_error = String::new();
            while Instant::now() < deadline {
                match self.assert_native_user_once(marker, images) {
                    Ok(()) => return Ok(()),
                    Err(error) => last_error = error,
                }
                thread::park_timeout(EVENT_POLL);
            }
            Err(format!(
                "timed out after {} seconds waiting for unique native input marker {marker:?}: {last_error}",
                timeout.as_secs()
            ))
        }

        pub fn history(&mut self) -> Result<Vec<Value>, String> {
            self.require_available("native history", &self.capabilities.sessions.history)?;
            if self.harness != Backend::Pi {
                return load_external_history(&self.path, self.project())
                    .ok_or_else(|| {
                        format!(
                            "E2E_BLOCKED: {} does not expose a native external history loader",
                            self.harness
                        )
                    })?
                    .map(|history| history.messages)
                    .map_err(|error| format!("load native {} history: {error}", self.harness));
            }
            let Payload::LoadHistory(SessionHistory::Replace { messages, .. }) =
                self.request(SessionCommand::LoadHistory)?
            else {
                return Err("expected replacement history".into());
            };
            Ok(messages)
        }

        pub fn replace_with_history(&mut self) -> Result<Vec<Value>, String> {
            let history = self.history()?;
            self.conversation.replace_history(&history);
            Ok(history)
        }

        pub fn history_reload(&mut self) -> Result<Vec<Value>, String> {
            self.reopen()?;
            self.replace_with_history()
        }

        pub fn reopen(&mut self) -> Result<(), String> {
            self.transport.close()?;
            let session_id = if self.harness == Backend::Pi {
                None
            } else {
                Some(
                    external_session_locator(self.harness, &self.path).ok_or_else(|| {
                        format!("invalid live session locator: {}", self.path.display())
                    })?,
                )
            };
            self.transport = spawn_session(
                &self.config,
                SessionLaunch {
                    harness: self.harness,
                    session_id,
                    project: self.project().into(),
                    start: SessionStart::Resume(self.path.clone()),
                    wake: Some(thread::current()),
                },
            )?;
            let reopened_path = self.session_path()?;
            if reopened_path != self.path {
                return Err(format!(
                    "resume forked or replaced live session: {} != {}",
                    reopened_path.display(),
                    self.path.display()
                ));
            }
            Ok(())
        }

        pub fn trace_summary(&self) -> String {
            self.activities
                .iter()
                .map(|event| format!("+{}ms {}", event.elapsed.as_millis(), event.value))
                .collect::<Vec<_>>()
                .join(" | ")
        }

        pub fn transcript_summary(&self) -> String {
            self.conversation
                .items
                .iter()
                .map(|item| format!("{:?}:{}", item.kind, item.complete_text()))
                .collect::<Vec<_>>()
                .join(" | ")
        }

        pub fn close_cleanup(mut self) -> Result<(), String> {
            let release = self
                .gates
                .iter()
                .filter(|gate| !gate.release_path.exists())
                .map(TurnGate::release)
                .collect::<Result<Vec<_>, _>>();
            let close = self.transport.close();
            let evidence = self.write_evidence();
            let cleanup = self.cleanup_locator();
            match (release, close, evidence, cleanup) {
                (Ok(_), Ok(()), Ok(()), Ok(())) => Ok(()),
                (release, close, evidence, cleanup) => Err([
                    release
                        .err()
                        .map(|error| format!("release live gate: {error}")),
                    close
                        .err()
                        .map(|error| format!("close live {} session: {error}", self.harness)),
                    evidence.err(),
                    cleanup.err(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("; ")),
            }
        }

        /// Closes a LoadState-only model probe.  It deliberately does not ask
        /// the native client to delete the locator: Codex rejects that request
        /// before a prompt has created a rollout.  Real E2E turns continue to
        /// use `close_cleanup`, which keeps deletion errors strict.
        fn close_model_probe(mut self) -> Result<(), String> {
            if self.submitted_prompt || !self.gates.is_empty() {
                return Err("refusing probe-only cleanup after a live prompt or gate".into());
            }
            let close = self.transport.close();
            let evidence = self.write_evidence();
            let identity = external_session_locator(self.harness, &self.path)
                .unwrap_or_else(|| self.path.display().to_string());
            eprintln!(
                "E2E_LIMIT: {} LoadState-only model probe retained no-turn native locator {identity}; skipping deletion because no persisted rollout was established",
                self.harness
            );
            match (close, evidence) {
                (Ok(()), Ok(())) => Ok(()),
                (close, evidence) => Err([
                    close
                        .err()
                        .map(|error| format!("close live {} model probe: {error}", self.harness)),
                    evidence.err(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("; ")),
            }
        }

        fn request(&mut self, command: SessionCommand) -> Result<Payload, String> {
            let operation = command.response_operation();
            let id = self.transport.send(command)?;
            let response = self.wait_for_response(&id, COMMAND_TIMEOUT)?;
            if response.operation() != operation {
                return Err(format!(
                    "command {id} returned {:?}, expected {operation:?}",
                    response.operation()
                ));
            }
            response.result.map_err(|error| error.to_string())
        }

        fn load_state(&mut self) -> Result<crate::extensions::SessionState, String> {
            let Payload::LoadState(state) = self.request(SessionCommand::LoadState)? else {
                return Err("expected state response".into());
            };
            Ok(*state)
        }

        fn pump_one(&mut self) -> Result<(), String> {
            match self.transport.poll() {
                Some(SessionEvent::Activity(event)) => {
                    let value = event.value().clone();
                    self.conversation.reduce(&value);
                    self.activities.push(TraceEvent {
                        elapsed: self.started_at.elapsed(),
                        value,
                    });
                }
                Some(SessionEvent::Response(response)) => {
                    if let Some(id) = response.id.clone() {
                        self.responses.insert(id, response);
                    }
                }
                Some(SessionEvent::Interaction(request)) => {
                    let mut allowed_commands = self.fixture_commands.clone();
                    allowed_commands.extend(self.gates.iter().map(TurnGate::shell_command));
                    approve(&mut *self.transport, request, &allowed_commands)?
                }
                Some(SessionEvent::Failure(error)) => {
                    return Err(format!("live {} transport failure: {error}", self.harness));
                }
                Some(SessionEvent::Stderr(line)) => self.stderr.push(line),
                None => thread::park_timeout(EVENT_POLL),
            }
            Ok(())
        }

        fn cleanup_locator(&self) -> Result<(), String> {
            if self.harness == Backend::Pi {
                return if self.path.is_file() {
                    fs::remove_file(&self.path).map_err(|error| {
                        format!("delete Pi live session {}: {error}", self.path.display())
                    })
                } else {
                    Ok(())
                };
            }
            if self.capabilities.sessions.delete != CapabilitySupport::Available {
                let identity = external_session_locator(self.harness, &self.path)
                    .unwrap_or_else(|| self.path.display().to_string());
                eprintln!(
                    "E2E_LIMIT: {} does not support native session deletion; retaining isolated native session {identity}",
                    self.harness
                );
                return Ok(());
            }
            let Some(delete) = delete_external_session(&self.path) else {
                return Err(format!(
                    "{} declares native session deletion but no test deletion path exists for {}",
                    self.harness,
                    self.path.display()
                ));
            };
            delete.map_err(|error| format!("delete live {} session: {error}", self.harness))
        }

        fn write_evidence(&self) -> Result<(), String> {
            let directory = std::env::var_os("FARCASTER_E2E_ARTIFACT_DIR")
                .map(PathBuf::from)
                .ok_or_else(|| "write E2E evidence without artifact directory".to_owned())?;
            let mut trace = fs::File::create(directory.join("activities.jsonl"))
                .map_err(|error| format!("create raw activity evidence: {error}"))?;
            for event in &self.activities {
                serde_json::to_writer(
                    &mut trace,
                    &json!({"elapsedMs": event.elapsed.as_millis(), "event": event.value}),
                )
                .map_err(|error| format!("encode raw activity evidence: {error}"))?;
                writeln!(trace).map_err(|error| format!("write raw activity evidence: {error}"))?;
            }
            fs::write(
                directory.join("session.json"),
                serde_json::to_vec_pretty(&json!({
                    "harness": &self.harness,
                    "programVersion": &self.program_version,
                    "model": &self.model_identity,
                    "sessionPath": &self.path,
                    "activityCount": self.activities.len(),
                    "stderr": self.stderr,
                }))
                .map_err(|error| format!("encode E2E session evidence: {error}"))?,
            )
            .map_err(|error| format!("write E2E session evidence: {error}"))
        }
    }

    pub fn selected_live_harnesses() -> Result<Vec<&'static str>, String> {
        select_harnesses(std::env::var("FARCASTER_E2E_HARNESS").ok().as_deref())
    }

    /// Resolves a child model from the installed harness's real `LoadState`.
    /// An explicit `FARCASTER_E2E_MODEL` must agree with that actual selection;
    /// this helper never asks for a catalog or changes a model.  The probe
    /// always closes before its caller can launch MCP-backed child workers,
    /// which restores the scoped MCP guard.
    pub fn selected_live_worker_model(harness: Backend) -> Result<LiveWorkerModel, String> {
        let mut probe = LiveSession::start(harness)?;
        let outcome = (|| {
            let state = probe.load_state()?;
            let model = state.model.ok_or_else(|| {
                format!(
                    "E2E_BLOCKED: {harness} LoadState omitted a selected model; refusing to choose an arbitrary catalog entry"
                )
            })?;
            if model.provider.trim().is_empty() || model.id.trim().is_empty() {
                return Err(format!(
                    "E2E_BLOCKED: {harness} LoadState reported an incomplete model identity provider={:?} model={:?}",
                    model.provider, model.id
                ));
            }
            if let Some(model_id) = std::env::var("FARCASTER_E2E_MODEL")
                .ok()
                .filter(|model_id| !model_id.trim().is_empty())
            {
                let matches_selected = model_id == model.id
                    || model.resolved_model.as_deref() == Some(model_id.as_str());
                if !matches_selected {
                    return Err(format!(
                        "FARCASTER_E2E_MODEL {model_id:?} differs from {harness} LoadState model {}/{}; this child probe will not guess from a catalog or send a model-selection request",
                        model.provider, model.id
                    ));
                }
            }
            Ok(LiveWorkerModel {
                provider: model.provider,
                model: model.id,
            })
        })();
        let cleanup = probe.close_model_probe();
        match (outcome, cleanup) {
            (Ok(model), Ok(())) => Ok(model),
            (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(cleanup)) => Err(format!("{error}; probe cleanup: {cleanup}")),
        }
    }

    pub fn e2e_case_dir() -> Result<PathBuf, String> {
        require_e2e_isolation()
    }

    pub fn isolated_locator_root() -> Result<PathBuf, String> {
        let data_dir = std::env::var_os("FARCASTER_DATA_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| "E2E_BLOCKED: missing FARCASTER_DATA_DIR".to_owned())?;
        let root = data_dir.join("session-locators");
        fs::create_dir_all(&root)
            .map_err(|error| format!("create isolated session locator root: {error}"))?;
        Ok(root)
    }

    pub fn live_access_mode(capabilities: &AgentCapabilities) -> Result<HarnessAccessMode, String> {
        match std::env::var("FARCASTER_E2E_ACCESS_MODE").ok().as_deref() {
            None | Some("sandboxed")
                if capabilities
                    .configuration
                    .access_modes
                    .contains(&HarnessAccessMode::Sandboxed) =>
            {
                Ok(HarnessAccessMode::Sandboxed)
            }
            // Full access requires a deliberate environment opt-in. Pi's
            // sandbox support comes from an optional adapter and therefore is
            // not listed in its static descriptor.
            Some("full") => Ok(HarnessAccessMode::Full),
            None | Some("sandboxed") => Err(
                "E2E_BLOCKED: selected harness has no sandboxed access mode; set FARCASTER_E2E_ACCESS_MODE=full only after approving a full-access live run"
                    .into(),
            ),
            Some(other) => Err(format!(
                "E2E_BLOCKED: unsupported FARCASTER_E2E_ACCESS_MODE {other:?}; expected sandboxed or full"
            )),
        }
    }

    pub fn live_access_mode_for_harness(harness: Backend) -> Result<HarnessAccessMode, String> {
        if harness == Backend::Pi {
            return match std::env::var("FARCASTER_E2E_ACCESS_MODE").ok().as_deref() {
                None | Some("sandboxed") => Ok(HarnessAccessMode::Sandboxed),
                Some("full") => Ok(HarnessAccessMode::Full),
                Some(other) => Err(format!(
                    "E2E_BLOCKED: unsupported FARCASTER_E2E_ACCESS_MODE {other:?}; expected sandboxed or full"
                )),
            };
        }
        live_access_mode(&descriptor(harness)?.capabilities)
    }

    pub fn native_questions_available(harness: Backend) -> Result<bool, String> {
        Ok(
            descriptor(harness)?.capabilities.interactions.questions
                == CapabilitySupport::Available,
        )
    }

    pub fn native_approvals_available(harness: Backend) -> Result<bool, String> {
        Ok(
            descriptor(harness)?.capabilities.interactions.approvals
                == CapabilitySupport::Available,
        )
    }

    pub fn require_native_input_support(harness: Backend) -> Result<(), String> {
        let capabilities = descriptor(harness)?.capabilities;
        if capabilities.interactions.questions == CapabilitySupport::Available
            || capabilities.interactions.approvals == CapabilitySupport::Available
        {
            Ok(())
        } else {
            Err(format!(
                "E2E_BLOCKED: {harness} declares both native questions and approvals unavailable"
            ))
        }
    }

    pub fn for_each_selected(
        mut exercise: impl FnMut(&mut LiveSession) -> Result<(), String>,
    ) -> Result<(), String> {
        for harness in selected_live_harnesses()? {
            let harness = harness.parse::<Backend>()?;
            let mut session = LiveSession::start(harness)
                .map_err(|error| format!("{harness}: start live session: {error}"))?;
            let outcome = exercise(&mut session).map_err(|error| format!("{harness}: {error}"));
            let cleanup = session.close_cleanup();
            match (outcome, cleanup) {
                (Ok(()), Ok(())) => {}
                (Err(error), Ok(())) | (Ok(()), Err(error)) => return Err(error),
                (Err(error), Err(cleanup)) => return Err(format!("{error}; cleanup: {cleanup}")),
            }
        }
        Ok(())
    }

    pub fn marker(label: &str) -> String {
        format!(
            "FARCASTER_E2E_{}_{}",
            label.to_ascii_uppercase(),
            uuid::Uuid::new_v4().simple()
        )
    }

    pub fn image(data: &str, mime_type: &str) -> PromptImage {
        PromptImage::new(data.into(), mime_type.into())
    }

    pub fn alternate_image() -> PromptImage {
        // Valid 1×1 GIF, distinct from TEST_IMAGE's PNG bytes.
        image(
            "R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==",
            "image/gif",
        )
    }

    pub fn new_turn_gate(project: &Path, label: &str) -> Result<TurnGate, String> {
        TurnGate::new(project, label)
    }

    fn require_e2e_isolation() -> Result<PathBuf, String> {
        let data_dir = std::env::var_os("FARCASTER_DATA_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| {
                "E2E_BLOCKED: live E2E requires FARCASTER_DATA_DIR from scripts/e2e.sh".to_owned()
            })?;
        let artifact_dir = std::env::var_os("FARCASTER_E2E_ARTIFACT_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| {
                "E2E_BLOCKED: live E2E requires FARCASTER_E2E_ARTIFACT_DIR from scripts/e2e.sh"
                    .to_owned()
            })?;
        let data_parent = data_dir.parent();
        let artifact_parent = artifact_dir.parent();
        if !data_dir.is_absolute()
            || !artifact_dir.is_absolute()
            || data_dir.file_name().and_then(|name| name.to_str()) != Some("data")
            || artifact_dir.file_name().and_then(|name| name.to_str()) != Some("evidence")
            || data_parent != artifact_parent
            || !data_dir.is_dir()
            || !artifact_dir.is_dir()
        {
            return Err(format!(
                "E2E_BLOCKED: live state/evidence must be sibling case data and evidence directories: data={}, evidence={}",
                data_dir.display(),
                artifact_dir.display()
            ));
        }
        Ok(data_parent.expect("checked matching parent").to_owned())
    }

    fn program_version(program: &Path) -> String {
        use std::process::{Command, Stdio};

        let Ok(mut child) = Command::new(program)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        else {
            return format!("unavailable ({})", program.display());
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match child.try_wait() {
                Ok(Some(_)) => match child.wait_with_output() {
                    Ok(output) => {
                        let text = String::from_utf8_lossy(&output.stdout);
                        let text = text.trim();
                        return if text.is_empty() {
                            String::from_utf8_lossy(&output.stderr).trim().to_owned()
                        } else {
                            text.lines().next().unwrap_or_default().to_owned()
                        };
                    }
                    Err(error) => return format!("read failed: {error}"),
                },
                Ok(None) => thread::sleep(EVENT_POLL),
                Err(error) => return format!("wait failed: {error}"),
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        "timed out after 5 seconds".into()
    }

    fn history_user_contains(message: &Value, marker: &str) -> bool {
        let role = message.get("role").and_then(Value::as_str).or_else(|| {
            message
                .get("message")
                .and_then(|message| message.get("role"))
                .and_then(Value::as_str)
        });
        role == Some("user") && message.to_string().contains(marker)
    }

    fn history_submission_id(message: &Value) -> Option<&str> {
        message
            .get("submissionId")
            .and_then(Value::as_str)
            .or_else(|| {
                message
                    .get("message")
                    .and_then(|message| message.get("submissionId"))
                    .and_then(Value::as_str)
            })
    }

    fn require_event_images(event: &Value, images: &[PromptImage]) -> Result<(), String> {
        let message = event.get("message").unwrap_or(&Value::Null);
        require_wire_images(message, images)
    }

    fn require_wire_images(message: &Value, images: &[PromptImage]) -> Result<(), String> {
        for image in images {
            let data = if image.data.is_empty() {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD.encode(image.bytes()?)
            } else {
                image.data.clone()
            };
            let wire = message.to_string();
            if !wire.contains(&data) || !wire.contains(&image.mime_type) {
                return Err(format!(
                    "record omitted exact image payload or MIME {}: {message}",
                    image.mime_type
                ));
            }
        }
        Ok(())
    }
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
