//! UI-neutral application runtime and active-session ownership.

mod access_mode;
mod catalog;
mod documents;
mod prompts;
mod session_controls;
mod session_identity;

pub(crate) use crate::agents::HarnessAccessMode;
use access_mode::AccessModeChangeState;
use prompts::DeferredPrompt;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant, SystemTime},
};

use serde_json::{Value, json};

use crate::{
    agent_activity::AgentActivity,
    agents::{
        self, AgentLaunchConfig, SessionActivityKind, SessionCommand, SessionEvent, SessionLaunch,
        SessionOperation, SessionStart, SessionTransport,
    },
    app::infrastructure::persistence::StateStore,
    app::views::transcript::conversation::{
        ConversationState, TranscriptItem, TranscriptKind, annotate_prompt_presentations,
    },
    protocol::{
        AgentMode, ExtensionUiRequest, ExtensionUiResponse, Model, PromptImage, PromptMode,
        SessionState, SlashCommand,
    },
    sessions::{
        self, ExternalActivityTracker, LoadedHistory, SessionDiscovery, SessionSummary,
        SessionWatchEvent, SessionWatcher, TransferMember, archived_root_family_for_path,
        configured_session_root, project_display_history, session_family_for_path,
    },
};
use session_controls::PendingSessionControls;
use session_identity::SessionControlDefaults;

const COALESCED_SESSION_REFRESH_DELAY: Duration = Duration::from_millis(100);
const STREAM_PUBLISH_INTERVAL: Duration = Duration::from_millis(16);
const MAX_FAILURE_DETAILS_CHARS: usize = 12_000;
const MAX_FAILURE_SUMMARY_CHARS: usize = 240;

mod supervisor;
mod types;

#[cfg(test)]
use documents::reconcile_live_session_documents;
pub(crate) use supervisor::RuntimeHandle;
use supervisor::{SessionEventSender, SessionRuntimeHandle};
#[cfg(test)]
use supervisor::{
    SupervisorSessionAction, UiEventSender, actor_key_for_command, changed_external_documents,
    initial_draft_command, is_view_only_selection, publish_session_status_if_changed,
    route_session_discovery, rpc_owned_session_paths, target_command_needs_actor_message,
};
pub(crate) use types::{RuntimeCommand, RuntimeEvent, RuntimeSnapshot};

fn stable_session_stats(previous: &Value, next: Value, running: bool) -> Value {
    if !running || context_usage_is_meaningful(&next) {
        return next;
    }
    let mut next = match next {
        Value::Object(next) => next,
        other => return other,
    };
    if let Some(context) = previous
        .get("contextUsage")
        .filter(|_| context_usage_is_meaningful(previous))
    {
        next.insert("contextUsage".into(), context.clone());
    } else {
        next.remove("contextUsage");
    }
    Value::Object(next)
}

fn context_usage_is_meaningful(stats: &Value) -> bool {
    let Some(context) = stats.get("contextUsage") else {
        return false;
    };
    context
        .get("tokens")
        .and_then(Value::as_u64)
        .is_some_and(|tokens| tokens > 0)
        || context
            .get("percent")
            .and_then(Value::as_f64)
            .is_some_and(|percent| percent.is_finite() && percent > 0.0)
}

fn historical_context_stats(messages: &[Value], models: &[Model]) -> Value {
    let Some(message) = messages.iter().rev().find(|message| {
        message.get("role").and_then(Value::as_str) == Some("assistant")
            && !matches!(
                message.get("stopReason").and_then(Value::as_str),
                Some("aborted" | "error")
            )
    }) else {
        return Value::Null;
    };
    let Some(usage) = message.get("usage") else {
        return Value::Null;
    };
    let tokens = usage
        .get("totalTokens")
        .and_then(Value::as_u64)
        .filter(|tokens| *tokens > 0)
        .or_else(|| {
            ["input", "output", "cacheRead", "cacheWrite"]
                .iter()
                .try_fold(0_u64, |total, key| {
                    total.checked_add(usage.get(*key)?.as_u64()?)
                })
                .filter(|tokens| *tokens > 0)
        });
    let Some(tokens) = tokens else {
        return Value::Null;
    };
    let provider = message.get("provider").and_then(Value::as_str);
    let model_id = message.get("model").and_then(Value::as_str);
    let context_window = models
        .iter()
        .find(|model| {
            Some(model.provider.as_str()) == provider && Some(model.id.as_str()) == model_id
        })
        .map(|model| model.context_window)
        .filter(|window| *window > 0);
    let mut context = json!({"tokens": tokens});
    if let Some(context_window) = context_window {
        context["contextWindow"] = context_window.into();
        context["percent"] = (tokens as f64 * 100.0 / context_window as f64).into();
    }
    json!({"contextUsage": context})
}

fn update_context_from_event(stats: &mut Value, event: &Value) -> bool {
    let usage = event
        .get("usage")
        .or_else(|| event.pointer("/message/usage"));
    let Some(usage) = usage else { return false };
    let tokens = usage
        .get("totalTokens")
        .and_then(Value::as_u64)
        .filter(|tokens| *tokens > 0)
        .or_else(|| {
            ["input", "output", "cacheRead", "cacheWrite"]
                .iter()
                .try_fold(0_u64, |total, key| {
                    total.checked_add(usage.get(*key)?.as_u64()?)
                })
                .filter(|tokens| *tokens > 0)
        });
    let Some(tokens) = tokens else { return false };
    let Some(context_window) = stats
        .pointer("/contextUsage/contextWindow")
        .and_then(Value::as_u64)
        .filter(|window| *window > 0)
    else {
        return false;
    };
    let percent = tokens as f64 * 100.0 / context_window as f64;
    if stats
        .pointer("/contextUsage/tokens")
        .and_then(Value::as_u64)
        == Some(tokens)
        && stats
            .pointer("/contextUsage/percent")
            .and_then(Value::as_f64)
            == Some(percent)
    {
        return false;
    }
    stats["contextUsage"]["tokens"] = tokens.into();
    stats["contextUsage"]["percent"] = percent.into();
    true
}

fn semantic_status(snapshot: &RuntimeSnapshot) -> &'static str {
    if snapshot.history_preview {
        return if snapshot.selected_session.is_none() {
            "Draft"
        } else {
            "Done"
        };
    }
    if snapshot.conversation.running {
        "Working"
    } else if snapshot
        .conversation
        .items
        .last()
        .is_some_and(|item| item.kind == TranscriptKind::Error)
    {
        "Failed"
    } else {
        "Done"
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SnapshotChange {
    None,
    Streaming,
    Immediate,
}

struct RuntimeOwner {
    project: PathBuf,
    harness: String,
    session_id: Option<String>,
    process_command: AgentLaunchConfig,
    process: Option<Box<dyn SessionTransport>>,
    snapshot: RuntimeSnapshot,
    owns_session_catalog: bool,
    session_generation: u64,
    session_discovery_in_flight: bool,
    session_refresh_pending: bool,
    session_refresh_due: Option<Instant>,
    process_generation: u64,
    pending_prompt_id: Option<String>,
    pending_prompt_target: Option<String>,
    pending_prompt_item: Option<Arc<TranscriptItem>>,
    pending_outbox_id: Option<i64>,
    transcript_changed_from: Option<usize>,
    event_tx: SessionEventSender,
    discovery_tx: mpsc::Sender<DiscoveryResult>,
    history_tx: mpsc::Sender<HistoryResult>,
    history_generation: u64,
    history_selection_generation: Option<u64>,
    document_refresh_generation: Option<u64>,
    pending_document_refresh: Option<(PathBuf, PathBuf)>,
    active_session: Option<PathBuf>,
    parked_snapshot: Option<RuntimeSnapshot>,
    deferred_prompt: Option<DeferredPrompt>,
    pending_session_controls: PendingSessionControls,
    access_mode_changes: AccessModeChangeState,
    startup_state_loaded: bool,
    startup_history_loaded: bool,
    state: Option<StateStore>,
    session_query: String,
}

fn import_agent_session(session: agents::DiscoveredSession) -> SessionSummary {
    SessionSummary::import(crate::sessions::SessionImport {
        id: session.id,
        harness: session.harness,
        path: session.path,
        project: session.project,
        title: session.title,
        first_user_message: session.first_user_message,
        timestamp: session.timestamp,
        parent_session: session.parent_session,
        modified: session.modified,
        message_count: session.message_count,
        usage: crate::sessions::UsageSummary {
            input: session.usage.input,
            output: session.usage.output,
            cache_read: session.usage.cache_read,
            cache_write: session.usage.cache_write,
            total: session.usage.total,
            cost_micros: session.usage.cost_micros,
        },
        archived: session.archived,
        is_running: session.is_running,
        search: session.search,
    })
}

fn import_agent_history(history: agents::DiscoveredHistory) -> LoadedHistory {
    LoadedHistory {
        messages: history.messages,
        model: history.model,
        thinking_level: history.thinking_level,
        pending_question: None,
    }
}

fn restored_question_request(question: crate::sessions::RestoredQuestion) -> ExtensionUiRequest {
    if question.options.is_empty() {
        ExtensionUiRequest::Input {
            id: question.id,
            title: question.title,
            placeholder: None,
            timeout: None,
        }
    } else {
        ExtensionUiRequest::Select {
            id: question.id,
            title: question.title,
            options: question.options,
            timeout: None,
        }
    }
}

fn load_session_history(path: &std::path::Path) -> Result<LoadedHistory, String> {
    agents::load_external_history(path)
        .map(|result| result.map(import_agent_history))
        .unwrap_or_else(|| sessions::load_history(path))
}

struct DiscoveryResult {
    generation: u64,
    result: Result<SessionDiscovery, String>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum HistoryLoadKind {
    Selection,
    DocumentRefresh,
}

struct HistoryResult {
    generation: u64,
    path: PathBuf,
    project: PathBuf,
    kind: HistoryLoadKind,
    result: Result<LoadedHistory, String>,
}

fn run(
    project: PathBuf,
    process_command: AgentLaunchConfig,
    command_rx: mpsc::Receiver<RuntimeCommand>,
    event_tx: SessionEventSender,
    load_catalog: bool,
) {
    let (discovery_tx, discovery_rx) = mpsc::channel();
    let (history_tx, history_rx) = mpsc::channel();
    let (watch_tx, watch_rx) = mpsc::channel();
    let (session_watcher, watcher_error) = if load_catalog {
        match configured_session_root()
            .and_then(|root| SessionWatcher::start(&root, watch_tx, thread::current()))
        {
            Ok(watcher) => (Some(watcher), None),
            Err(error) => (None, Some(error)),
        }
    } else {
        (None, None)
    };
    let (state, state_error) = match StateStore::open() {
        Ok(state) => (Some(state), None),
        Err(error) => (None, Some(error)),
    };
    let mut owner = RuntimeOwner {
        project: project.clone(),
        harness: "pi".into(),
        session_id: None,
        process_command,
        process: None,
        snapshot: RuntimeSnapshot {
            status: "Done".into(),
            project,
            auto_retry: true,
            ..RuntimeSnapshot::default()
        },
        owns_session_catalog: load_catalog,
        session_generation: 0,
        session_discovery_in_flight: false,
        session_refresh_pending: false,
        session_refresh_due: None,
        process_generation: 0,
        pending_prompt_id: None,
        pending_prompt_target: None,
        pending_prompt_item: None,
        pending_outbox_id: None,
        transcript_changed_from: Some(0),
        event_tx,
        discovery_tx,
        history_tx,
        history_generation: 0,
        history_selection_generation: None,
        document_refresh_generation: None,
        pending_document_refresh: None,
        active_session: None,
        parked_snapshot: None,
        deferred_prompt: None,
        pending_session_controls: PendingSessionControls::default(),
        access_mode_changes: AccessModeChangeState::default(),
        startup_state_loaded: false,
        startup_history_loaded: false,
        state,
        session_query: String::new(),
    };
    if let Some(error) = state_error {
        conversation_mut(&mut owner.snapshot).push_local_error("State unavailable", error);
    }
    if load_catalog {
        owner.load_sessions(String::new());
    }
    if let Some(message) = watcher_error {
        let _ = owner.event_tx.send(RuntimeEvent::SessionsFailed {
            generation: owner.session_generation,
            message,
        });
    }
    owner.publish();
    let _session_watcher = session_watcher;
    let mut running = true;
    let mut stream_publish_due = None;
    while running {
        while let Ok(result) = discovery_rx.try_recv() {
            owner.apply_discovery(result);
        }
        while let Ok(result) = history_rx.try_recv() {
            owner.apply_history(result);
        }
        while let Ok(event) = watch_rx.try_recv() {
            match event {
                SessionWatchEvent::CatalogChanged => owner.schedule_session_refresh(),
                SessionWatchEvent::Activity(paths) => {
                    let _ = owner
                        .event_tx
                        .send(RuntimeEvent::SessionFilesModified { paths });
                }
                SessionWatchEvent::Failed(message) => {
                    let _ = owner.event_tx.send(RuntimeEvent::SessionsFailed {
                        generation: owner.session_generation,
                        message,
                    });
                }
            }
        }
        owner.poll_deferred_session_refresh(Instant::now());
        let mut immediate_snapshot_change = false;
        while let Some(item) = owner.process.as_mut().and_then(|process| process.poll()) {
            match owner.apply_process_item(item) {
                SnapshotChange::None => {}
                SnapshotChange::Streaming => {
                    let coalesced = stream_publish_due.is_some();
                    crate::app::infrastructure::performance::count_stream_event(coalesced);
                    if !coalesced {
                        stream_publish_due = Some(Instant::now() + STREAM_PUBLISH_INTERVAL);
                    }
                }
                SnapshotChange::Immediate => immediate_snapshot_change = true,
            }
        }
        owner.apply_queued_access_mode_change();
        if immediate_snapshot_change
            || stream_publish_due.is_some_and(|deadline| Instant::now() >= deadline)
        {
            owner.publish();
            stream_publish_due = None;
        }
        let now = Instant::now();
        let access_mode_change_due = owner
            .access_mode_change_ready()
            .then(|| owner.access_mode_changes.next_deadline())
            .flatten();
        let next_deadline = [
            stream_publish_due,
            owner.session_refresh_due,
            access_mode_change_due,
        ]
        .into_iter()
        .flatten()
        .min();
        match command_rx.try_recv() {
            Ok(RuntimeCommand::Shutdown) => running = false,
            Ok(command) => owner.apply_command(command),
            Err(mpsc::TryRecvError::Empty) => match next_deadline {
                Some(deadline) => thread::park_timeout(deadline.saturating_duration_since(now)),
                None => thread::park(),
            },
            Err(mpsc::TryRecvError::Disconnected) => running = false,
        }
    }
    if let Some(mut process) = owner.process.take() {
        let _ = process.close();
    }
    let _ = owner.event_tx.send(RuntimeEvent::Stopped);
}

impl RuntimeOwner {
    pub(super) fn backend_name(&self) -> &str {
        match self.harness.as_str() {
            "pi" => "Pi",
            "codex-cli" => "Codex",
            "opencode2" => "OpenCode",
            other => other,
        }
    }

    fn start_process(&mut self, session: Option<PathBuf>) {
        self.start_process_from(session, None, false);
    }

    fn restart_process_preserving_transcript(&mut self) {
        let session = if self.snapshot.history_preview {
            self.snapshot.selected_session.clone()
        } else {
            self.active_session.clone()
        };
        self.start_process_from(session, None, true);
    }

    fn start_fork_process(&mut self, source: PathBuf) {
        self.start_process_from(None, Some(source), false);
    }

    fn reset_process_runtime(&mut self) {
        self.invalidate_history_loads();
        self.process_generation = self.process_generation.saturating_add(1);
        if let Some(mut process) = self.process.take() {
            let _ = process.close();
        }
        self.active_session = None;
        self.parked_snapshot = None;
        self.startup_state_loaded = false;
        self.startup_history_loaded = false;
        self.pending_prompt_id = None;
        self.pending_prompt_item = None;
        self.transcript_changed_from = Some(0);
    }

    fn start_process_from(
        &mut self,
        session: Option<PathBuf>,
        fork: Option<PathBuf>,
        preserve_transcript: bool,
    ) {
        let preserve_transcript = preserve_transcript
            || self.deferred_prompt.is_some()
            || (!self.pending_session_controls.is_empty() && self.snapshot.history_preview);
        let keep_preview = preserve_transcript && self.snapshot.history_preview;
        let preserved_conversation =
            (preserve_transcript && !keep_preview).then(|| self.snapshot.conversation.clone());
        let preserved_prompt_item = preserved_conversation
            .as_ref()
            .and(self.pending_prompt_item.clone());
        self.reset_process_runtime();
        self.active_session = session.clone();
        self.process_command.access_mode = self
            .access_mode_changes
            .take_requested_mode(self.process_command.access_mode);
        let status = if fork.is_some() {
            "Forking session".into()
        } else {
            session.as_ref().map_or_else(
                || "Starting new session".into(),
                |_| "Resuming session".into(),
            )
        };
        if keep_preview {
            let mut loading = RuntimeSnapshot {
                auto_retry: self.snapshot.auto_retry,
                ..RuntimeSnapshot::default()
            };
            reset_snapshot_for_process(&mut loading, self.project.clone(), session.clone(), status);
            self.parked_snapshot = Some(loading);
        } else {
            reset_snapshot_for_process(
                &mut self.snapshot,
                self.project.clone(),
                session.clone(),
                status,
            );
            if let Some(conversation) = preserved_conversation {
                self.snapshot.conversation = conversation;
                self.pending_prompt_item = preserved_prompt_item;
            }
        }
        let _ = self.event_tx.send(RuntimeEvent::SessionReset {
            generation: self.process_generation,
            preserve_submission: preserve_transcript,
        });
        self.publish();
        let start = if let Some(source) = fork {
            SessionStart::Fork(source)
        } else if let Some(session) = session {
            SessionStart::Resume(session)
        } else {
            SessionStart::New
        };
        self.process_command.access_mode =
            crate::agents::normalize_access_mode(&self.harness, self.process_command.access_mode);
        let process = crate::agents::spawn_session(
            &self.process_command,
            SessionLaunch {
                harness: self.harness.clone(),
                session_id: self.session_id.clone(),
                project: self.project.clone(),
                start,
                wake: Some(thread::current()),
            },
        );
        match process {
            Ok(process) => {
                self.process = Some(process);
                let snapshot = self.active_snapshot_mut();
                snapshot.connected = true;
                snapshot.status = "Loading session".into();
                self.send_startup_queries();
            }
            Err(error) => self.fail(error),
        }
        self.publish();
    }

    fn send_startup_queries(&mut self) {
        for command in startup_commands() {
            if agents::supports_startup_command(&self.harness, &command) {
                self.send(command);
            }
        }
    }

    fn apply_command(&mut self, runtime_command: RuntimeCommand) {
        match runtime_command {
            RuntimeCommand::Prompt {
                target,
                mode,
                message,
                display_message,
                invocation,
                images,
                allow_while_running,
            } => match (display_message, invocation) {
                (None, None) => {
                    self.send_prompt(target, mode, message, images, allow_while_running)
                }
                (display_message, invocation) => self.send_prompt_with_presentation(
                    target,
                    mode,
                    message,
                    display_message,
                    invocation,
                    images,
                    allow_while_running,
                ),
            },
            RuntimeCommand::DeliverQueued(prompt) => self.deliver_queued(prompt),
            RuntimeCommand::Abort => self.send(SessionCommand::Abort),
            RuntimeCommand::Reload => self.reload(),
            RuntimeCommand::Compact {
                custom_instructions,
            } => self.send(SessionCommand::Compact {
                instructions: custom_instructions,
            }),
            RuntimeCommand::ExportHtml { output_path } => {
                self.send(SessionCommand::ExportHtml { output_path })
            }
            RuntimeCommand::SetSessionName(name) => {
                if let Some(state) = self.active_snapshot_mut().session.as_mut() {
                    state.session_name = Some(name.clone());
                }
                self.send(SessionCommand::Rename { name })
            }
            RuntimeCommand::RenameSession {
                path,
                harness,
                session_id,
                project,
                name,
            } => {
                match crate::agents::rename_session(
                    &self.process_command,
                    &harness,
                    &project,
                    &path,
                    &session_id,
                    &name,
                ) {
                    Ok(()) => self.load_sessions(self.session_query.clone()),
                    Err(message) => {
                        let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                            generation: self.session_generation,
                            message,
                        });
                    }
                }
            }
            RuntimeCommand::MoveSession { .. }
            | RuntimeCommand::StopSessionFamily { .. }
            | RuntimeCommand::DeleteSessionFamily { .. } => {}
            RuntimeCommand::NewSession {
                harness, project, ..
            } => self.stage_draft(harness, project),
            RuntimeCommand::ForkSession {
                path,
                harness,
                session_id,
                project,
            } => {
                self.project = project;
                self.harness = harness;
                self.session_id = Some(session_id);
                self.start_fork_process(path);
            }
            RuntimeCommand::ResumeDraft {
                harness, project, ..
            } => self.stage_draft(harness, project),
            RuntimeCommand::SelectSession {
                path,
                harness,
                session_id,
                project,
            } => {
                self.harness = harness;
                self.session_id = Some(session_id);
                self.select_history(path, project);
            }
            RuntimeCommand::RestartSession {
                path,
                harness,
                session_id,
                project,
            } => {
                self.project = project;
                self.harness = harness;
                self.session_id = Some(session_id);
                self.start_process(Some(path));
            }
            RuntimeCommand::RefreshSessionDocument { path, project } => {
                self.refresh_session_document(path, project)
            }
            RuntimeCommand::SetModel(model) => self.set_model(model),
            RuntimeCommand::SetThinking(level) => self.set_thinking(level),
            RuntimeCommand::SetMode(mode) => self.send(SessionCommand::SelectMode { mode }),
            RuntimeCommand::SetAccessMode(mode) => self.set_access_mode(mode),
            RuntimeCommand::SetAppProxy(proxy) => self.set_app_proxy(proxy),
            RuntimeCommand::ExtensionResponse(response) => {
                if let Some(process) = self.process.as_mut()
                    && let Err(error) = process.respond(response)
                {
                    self.fail(error);
                }
            }
            RuntimeCommand::SetSessionArchived { path, archived } => {
                if let Some(state) = &self.state
                    && let Err(error) = sessions::set_archived(state, &path, archived)
                {
                    let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                        generation: self.session_generation,
                        message: error,
                    });
                }
                self.load_sessions(self.session_query.clone());
            }
            RuntimeCommand::LoadSessions(query) => self.load_sessions(query),
            RuntimeCommand::RefreshSessions => self.refresh_sessions(),
            RuntimeCommand::Shutdown => {}
        }
    }

    fn reload(&mut self) {
        let active = self.active_snapshot();
        if active.conversation.running || active.conversation.compacting {
            let snapshot = self.active_snapshot_mut();
            conversation_mut(snapshot).push_local_error(
                "Reload not started",
                "Wait for the current response to finish before reloading.".into(),
            );
            snapshot.status = "Reload not started".into();
            self.publish();
            return;
        }
        let session = if self.snapshot.history_preview {
            self.snapshot.selected_session.clone()
        } else {
            self.active_session.clone()
        };
        self.start_process(session);
    }

    fn send(&mut self, request: SessionCommand) {
        let operation = request.operation();
        match self.process.as_mut().map(|process| process.send(request)) {
            Some(Ok(_)) => {}
            Some(Err(error)) => self.fail(error),
            None => self.fail(format!(
                "Cannot {operation}: {} is not connected",
                self.backend_name()
            )),
        }
    }

    fn apply_process_item(&mut self, item: SessionEvent) -> SnapshotChange {
        match item {
            SessionEvent::Response(response) => {
                self.apply_response(response);
                SnapshotChange::None
            }
            SessionEvent::Interaction(request) => {
                let _ = self.event_tx.send(RuntimeEvent::ExtensionUi {
                    generation: self.process_generation,
                    request,
                    system_notification_target: None,
                });
                SnapshotChange::None
            }
            SessionEvent::Activity(event) => {
                let settled = event.kind() == &SessionActivityKind::AgentSettled;
                let session_starting = event.kind() == &SessionActivityKind::AgentStarted
                    && self.active_session.is_none()
                    && self.parked_snapshot.is_none();
                let previewing = self.parked_snapshot.is_some();
                let previous_live_status =
                    previewing.then(|| session_badge_status(&self.active_snapshot().conversation));
                let (changed_from, snapshot_changed, live_status_changed) = {
                    let snapshot = self.active_snapshot_mut();
                    let (changed_from, conversation_state_changed) =
                        conversation_mut(snapshot).reduce_deferred_with_change(event.value());
                    let context_changed =
                        update_context_from_event(&mut snapshot.stats, event.value());
                    let status = run_status(&snapshot.conversation);
                    let status_changed = snapshot.status != status;
                    snapshot.status = status.to_owned();
                    let live_status_changed = previous_live_status.is_some_and(|status| {
                        status != session_badge_status(&snapshot.conversation)
                    });
                    (
                        changed_from,
                        changed_from.is_some()
                            || conversation_state_changed
                            || context_changed
                            || status_changed,
                        live_status_changed,
                    )
                };
                if let Some(changed_from) = changed_from {
                    self.transcript_changed_from = Some(
                        self.transcript_changed_from
                            .map_or(changed_from, |previous| previous.min(changed_from)),
                    );
                }
                let should_publish = (!previewing && snapshot_changed) || live_status_changed;
                if session_starting {
                    self.send(SessionCommand::LoadState);
                }
                if event.kind() == &SessionActivityKind::AgentStarted {
                    self.refresh_sessions();
                }
                if event.kind() == &SessionActivityKind::SessionChanged {
                    self.send(SessionCommand::LoadState);
                    self.refresh_sessions();
                }
                if settled {
                    self.send(SessionCommand::LoadState);
                    self.send(SessionCommand::LoadUsage);
                    self.refresh_sessions();
                }
                if !should_publish {
                    SnapshotChange::None
                } else if matches!(
                    event.kind(),
                    SessionActivityKind::MessageUpdated | SessionActivityKind::ToolUpdated
                ) {
                    SnapshotChange::Streaming
                } else {
                    SnapshotChange::Immediate
                }
            }
            SessionEvent::Stderr(chunk) => {
                let previewing = self.parked_snapshot.is_some();
                let snapshot = self.active_snapshot_mut();
                snapshot.stderr.push_str(&chunk);
                if snapshot.stderr.len() > 32 * 1024 {
                    snapshot.stderr.drain(..16 * 1024);
                }
                if previewing {
                    SnapshotChange::None
                } else {
                    SnapshotChange::Streaming
                }
            }
            SessionEvent::Failure(error) => {
                self.fail(error);
                SnapshotChange::None
            }
        }
    }

    fn active_snapshot_mut(&mut self) -> &mut RuntimeSnapshot {
        self.parked_snapshot.as_mut().unwrap_or(&mut self.snapshot)
    }

    fn active_snapshot(&self) -> &RuntimeSnapshot {
        self.parked_snapshot.as_ref().unwrap_or(&self.snapshot)
    }

    fn select_history(&mut self, path: PathBuf, project: PathBuf) {
        let _timing =
            crate::app::infrastructure::performance::Timing::new("switch.select_document");
        self.history_generation = self.history_generation.saturating_add(1);
        self.pending_document_refresh = None;
        if self.snapshot.selected_session.as_deref() == Some(path.as_path())
            && (self.snapshot.history_preview || self.process.is_some())
        {
            return;
        }
        if self.active_session.as_deref() == Some(path.as_path())
            && (self.process.is_some() || self.parked_snapshot.is_some())
        {
            if let Some(snapshot) = self.parked_snapshot.take() {
                self.snapshot = snapshot;
                self.project = project;
                let _ = self.event_tx.send(RuntimeEvent::HistoryReset {
                    generation: self.process_generation,
                });
                self.publish();
            }
            return;
        }
        self.refresh_history(path, project, HistoryLoadKind::Selection);
    }

    fn refresh_session_document(&mut self, path: PathBuf, project: PathBuf) {
        if self.active_session.as_deref() == Some(path.as_path()) && self.process.is_some() {
            return;
        }
        if self.history_selection_generation.is_some() {
            self.pending_document_refresh = Some((path, project));
            return;
        }
        if self.document_refresh_generation.is_some() {
            self.pending_document_refresh = Some((path, project));
            return;
        }
        self.refresh_history(path, project, HistoryLoadKind::DocumentRefresh);
    }

    fn refresh_history(&mut self, path: PathBuf, project: PathBuf, kind: HistoryLoadKind) {
        self.history_generation = self.history_generation.saturating_add(1);
        let generation = self.history_generation;
        *self.history_load_generation_mut(kind) = Some(generation);
        let sender = self.history_tx.clone();
        let wake = thread::current();
        let failed_path = path.clone();
        let failed_project = project.clone();
        if let Err(error) = thread::Builder::new()
            .name("farcaster-history".into())
            .spawn(move || {
                let _timing =
                    crate::app::infrastructure::performance::Timing::new("switch.load_history");
                let mut operation = crate::app::infrastructure::performance::OperationTiming::new(
                    crate::app::infrastructure::performance::OperationKind::HistoryLoad,
                    0,
                );
                let result = load_session_history(&path);
                if let Ok(history) = &result {
                    operation.set_work(history.messages.len());
                }
                let _ = sender.send(HistoryResult {
                    generation,
                    path,
                    project,
                    kind,
                    result,
                });
                wake.unpark();
            })
        {
            self.apply_history(HistoryResult {
                generation,
                path: failed_path,
                project: failed_project,
                kind,
                result: Err(format!("start session history load: {error}")),
            });
        }
    }

    fn history_load_generation_mut(&mut self, kind: HistoryLoadKind) -> &mut Option<u64> {
        match kind {
            HistoryLoadKind::Selection => &mut self.history_selection_generation,
            HistoryLoadKind::DocumentRefresh => &mut self.document_refresh_generation,
        }
    }

    fn invalidate_history_loads(&mut self) {
        self.history_generation = self.history_generation.saturating_add(1);
        self.history_selection_generation = None;
        self.document_refresh_generation = None;
        self.pending_document_refresh = None;
    }

    /// Configure an unsubmitted draft without starting its backend.
    fn stage_draft(&mut self, harness: String, project: PathBuf) {
        let unchanged = self.process.is_none()
            && self.parked_snapshot.is_none()
            && !self.snapshot.history_preview
            && self.harness == harness
            && self.project == project;
        if unchanged {
            self.publish();
            return;
        }

        self.reset_process_runtime();
        self.harness = harness;
        self.project = project.clone();
        self.session_id = None;
        self.pending_prompt_target = None;
        self.pending_outbox_id = None;
        self.deferred_prompt = None;
        self.pending_session_controls = PendingSessionControls::default();
        reset_snapshot_for_process(&mut self.snapshot, project, None, "Ready".into());
        self.publish();
    }

    fn apply_history(&mut self, result: HistoryResult) {
        let active_generation = self.history_load_generation_mut(result.kind);
        if *active_generation == Some(result.generation) {
            *active_generation = None;
        }
        if result.generation != self.history_generation {
            self.start_pending_document_refresh();
            return;
        }
        let refreshing_visible_history = result.kind == HistoryLoadKind::DocumentRefresh
            && self.snapshot.history_preview
            && self.snapshot.selected_session.as_ref() == Some(&result.path);
        let mut history = match result.result {
            Ok(history) => history,
            Err(error) => {
                self.snapshot.status = "Could not load history".into();
                conversation_mut(&mut self.snapshot).push_local_error("History unavailable", error);
                self.publish();
                self.start_pending_document_refresh();
                return;
            }
        };
        annotate_history_presentations(self.state.as_ref(), &result.path, &mut history.messages);
        if self.parked_snapshot.is_none() {
            self.parked_snapshot = Some(std::mem::take(&mut self.snapshot));
        }
        self.project = result.project.clone();
        let parked = self.parked_snapshot.as_ref();
        let auto_retry = parked.is_some_and(|snapshot| snapshot.auto_retry);
        let models = parked
            .map(|snapshot| snapshot.models.clone())
            .unwrap_or_default();
        let stats = historical_context_stats(&history.messages, &models);
        let prefill_model = SessionControlDefaults::history_model(&models, history.model.as_ref());
        let mut conversation = ConversationState::default();
        conversation.replace_history(&history.messages);
        self.transcript_changed_from = Some(0);
        self.snapshot = RuntimeSnapshot {
            connected: true,
            status: "Ready".into(),
            project: result.project,
            selected_session: Some(result.path),
            conversation: Arc::new(conversation),
            models,
            stats,
            auto_retry,
            history_preview: true,
            pending_question: history.pending_question.map(restored_question_request),
            prefill_model,
            prefill_thinking_level: history.thinking_level,
            ..RuntimeSnapshot::default()
        };
        if !refreshing_visible_history {
            let _ = self.event_tx.send(RuntimeEvent::HistoryReset {
                generation: self.process_generation,
            });
        }
        self.publish();
        self.start_pending_document_refresh();
    }

    fn start_pending_document_refresh(&mut self) {
        if self.history_selection_generation.is_some() || self.document_refresh_generation.is_some()
        {
            return;
        }
        if let Some((path, project)) = self.pending_document_refresh.take()
            && self.snapshot.history_preview
            && self.snapshot.selected_session.as_ref() == Some(&path)
        {
            self.refresh_session_document(path, project);
        }
    }

    fn apply_response(&mut self, response: crate::agents::SessionResponse) {
        let operation = response.operation;
        let is_prompt_response = matches!(operation, SessionOperation::Prompt(_))
            && response.id.as_ref() == self.pending_prompt_id.as_ref();
        if is_prompt_response {
            self.pending_prompt_id = None;
            if response.success {
                let target = self.pending_prompt_target.clone().unwrap_or_default();
                let session = self.active_session.clone();
                if let Some(id) = self.pending_outbox_id.take()
                    && let Some(state) = self.state.as_mut()
                {
                    let _ = agents::complete_prompt(state, id, &target, session.as_deref());
                }
            } else {
                self.mark_outbox_failed(
                    response
                        .error
                        .as_deref()
                        .unwrap_or("The harness rejected the prompt"),
                );
            }
            if response.success {
                self.pending_prompt_item = None;
            } else {
                self.rollback_pending_prompt();
            }
            if let Some(target) = self.pending_prompt_target.take() {
                self.emit_prompt_result(&target, response.success);
            }
        }
        if !response.success {
            let startup_query = matches!(
                operation,
                SessionOperation::LoadState | SessionOperation::LoadHistory
            );
            let blocks_resume = self.deferred_prompt.is_some() && startup_query;
            let blocks_session_command_resume =
                !self.pending_session_controls.is_empty() && startup_query;
            if blocks_session_command_resume {
                let details = format!(
                    "{operation:?}: {}",
                    response.error.unwrap_or_else(|| "command failed".into())
                );
                self.fail_session_control_resume("Command not sent", "Command not sent", details);
                return;
            }
            let snapshot = self.active_snapshot_mut();
            conversation_mut(snapshot).push_local_error(
                "Command failed",
                format!(
                    "{operation:?}: {}",
                    response.error.unwrap_or_else(|| "command failed".into())
                ),
            );
            snapshot.status = "Command failed".into();
            if blocks_resume {
                self.rollback_pending_prompt();
                self.deferred_prompt = None;
                if let Some(target) = self.pending_prompt_target.take() {
                    self.emit_prompt_result(&target, false);
                }
                if let Some(snapshot) = self.parked_snapshot.take() {
                    self.snapshot = snapshot;
                }
            }
            if self.parked_snapshot.is_none() {
                self.publish();
            }
            return;
        }
        match operation {
            SessionOperation::LoadState => {
                match serde_json::from_value::<SessionState>(response.data) {
                    Ok(state) => {
                        let previous_session = self.active_session.clone();
                        let selected_session = state
                            .session_file
                            .as_ref()
                            .map(PathBuf::from)
                            .map(|path| crate::sessions::normalize_session_path(&path))
                            .or_else(|| self.active_session.clone());
                        self.active_session = selected_session.clone();
                        let snapshot = self.active_snapshot_mut();
                        snapshot.selected_session = selected_session;
                        conversation_mut(snapshot).running = state.is_streaming;
                        snapshot.session = Some(state);
                        snapshot.status = "Ready".into();
                        self.startup_state_loaded = true;
                        if self.active_session.is_some() && self.active_session != previous_session
                        {
                            self.refresh_sessions();
                        }
                    }
                    Err(error) => {
                        self.fail(format!("decode get_state: {error}"));
                        return;
                    }
                }
            }
            SessionOperation::LoadHistory => {
                if response.data.get("preserve").and_then(Value::as_bool) != Some(true) {
                    let entries = response
                        .data
                        .get("entries")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let mut messages = project_display_history(&entries);
                    if let (Some(state), Some(session)) =
                        (self.state.as_ref(), self.active_session.as_deref())
                    {
                        annotate_history_presentations(Some(state), session, &mut messages);
                    }
                    conversation_mut(self.active_snapshot_mut()).replace_history(&messages);
                }
                self.startup_history_loaded = true;
            }
            SessionOperation::ListModels => {
                self.active_snapshot_mut().models = response
                    .data
                    .get("models")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default();
            }
            SessionOperation::ListReasoningLevels => {
                self.active_snapshot_mut().thinking_levels = response
                    .data
                    .get("levels")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
            }
            SessionOperation::ListModes => {
                self.active_snapshot_mut().modes = response
                    .data
                    .get("modes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|mode| serde_json::from_value(mode.clone()).ok())
                    .collect();
                self.active_snapshot_mut().selected_mode = response
                    .data
                    .get("selected")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            SessionOperation::LoadUsage => {
                let running = self.active_snapshot().conversation.running;
                let previous = self.active_snapshot().stats.clone();
                self.active_snapshot_mut().stats =
                    stable_session_stats(&previous, response.data, running);
            }
            SessionOperation::ListCommands => {
                self.active_snapshot_mut().commands = response
                    .data
                    .get("commands")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|command| serde_json::from_value(command.clone()).ok())
                    .collect()
            }
            SessionOperation::SelectModel => {
                if let Ok(model) = serde_json::from_value::<Model>(response.data)
                    && let Some(state) = self.active_snapshot_mut().session.as_mut()
                {
                    state.model = Some(model);
                }
                self.send(SessionCommand::ListReasoningLevels);
                self.send(SessionCommand::LoadState);
            }
            SessionOperation::SelectReasoning => {
                self.send(SessionCommand::LoadState);
            }
            SessionOperation::SelectMode => {
                self.send(SessionCommand::ListModes);
                self.send(SessionCommand::LoadState);
            }
            SessionOperation::Prompt(_) => {
                self.active_snapshot_mut().status = "Accepted".into();
                self.send(SessionCommand::LoadState);
            }
            SessionOperation::Abort => self.active_snapshot_mut().status = "Stopping".into(),
            SessionOperation::Compact => self.send(SessionCommand::LoadState),
            SessionOperation::Rename => {
                self.active_snapshot_mut().status = "Session named".into();
                self.send(SessionCommand::LoadState);
                self.refresh_sessions();
            }
            SessionOperation::ExportHtml => {
                self.active_snapshot_mut().status = response
                    .data
                    .get("path")
                    .and_then(Value::as_str)
                    .map_or_else(
                        || "Session exported".into(),
                        |path| format!("Exported to {path}"),
                    );
            }
            _ => {}
        }
        if matches!(
            operation,
            SessionOperation::LoadState | SessionOperation::LoadHistory
        ) {
            self.maybe_send_pending_session_controls();
            self.maybe_send_deferred_prompt();
        }
        if self.parked_snapshot.is_none() {
            self.publish();
        }
    }

    fn rollback_pending_prompt(&mut self) {
        if let Some(optimistic) = self.pending_prompt_item.take() {
            conversation_mut(self.active_snapshot_mut()).rollback_local_user(&optimistic);
        }
    }

    fn fail(&mut self, error: String) {
        let starting = !self.startup_state_loaded || !self.startup_history_loaded;
        let preserve_history = !self.pending_session_controls.is_empty()
            && self.snapshot.history_preview
            && self.parked_snapshot.is_some();
        let details = failure_details(&error);
        zlog::error!("agent runtime failed: {details}");
        self.mark_outbox_failed(&details);
        self.pending_prompt_id = None;
        self.deferred_prompt = None;
        self.process_command.access_mode = self
            .access_mode_changes
            .take_requested_mode(self.process_command.access_mode);
        self.rollback_pending_prompt();
        if let Some(target) = self.pending_prompt_target.take() {
            self.emit_prompt_result(&target, false);
        }
        if preserve_history {
            let label = format!("Couldn’t start {}", self.backend_name());
            self.fail_session_control_resume("Failed", &label, details);
            return;
        }
        self.pending_session_controls = PendingSessionControls::default();
        if let Some(mut process) = self.process.take() {
            let _ = process.close();
        }
        let previewing = self.parked_snapshot.is_some();
        let label = if starting {
            format!("Couldn’t start {}", self.backend_name())
        } else {
            format!("{} stopped", self.backend_name())
        };
        let snapshot = self.active_snapshot_mut();
        snapshot.connected = false;
        snapshot.status = "Failed".into();
        let conversation = conversation_mut(snapshot);
        conversation.diagnostics.push(details.clone());
        conversation.push_local_error_with_details(&label, failure_summary(&details), details);
        if previewing && let Some(snapshot) = self.parked_snapshot.take() {
            self.snapshot = snapshot;
        }
        self.publish();
    }

    fn mark_outbox_failed(&mut self, error: &str) {
        if let Some(id) = self.pending_outbox_id.take()
            && let Some(state) = &self.state
            && let Err(database_error) = agents::fail_prompt(state, id, error)
        {
            zlog::error!("Failed to mark queued prompt {id} as failed: {database_error}");
        }
    }

    fn publish(&mut self) {
        crate::app::infrastructure::performance::count_snapshot();
        self.snapshot.access_mode = self
            .access_mode_changes
            .requested_mode(self.process_command.access_mode);
        conversation_mut(self.active_snapshot_mut()).flush_live_projection();
        let active_snapshot = self.active_snapshot();
        let mut snapshot = self.snapshot.clone();
        snapshot.harness.clone_from(&self.harness);
        snapshot.live_session = self
            .active_session
            .clone()
            .or_else(|| active_snapshot.selected_session.clone());
        snapshot.live_status = session_badge_status(&active_snapshot.conversation).into();
        snapshot.transcript_changed_from = self.transcript_changed_from.take();
        let _ = self.event_tx.send(RuntimeEvent::Snapshot {
            generation: self.process_generation,
            snapshot: Arc::new(snapshot),
        });
    }
}

fn annotate_history_presentations(
    state: Option<&StateStore>,
    session: &std::path::Path,
    messages: &mut [Value],
) {
    let Some(state) = state else { return };
    if let Ok(presentations) = state.prompt_presentations(session) {
        annotate_prompt_presentations(messages, &presentations);
    }
}

pub(super) fn conversation_mut(snapshot: &mut RuntimeSnapshot) -> &mut ConversationState {
    Arc::make_mut(&mut snapshot.conversation)
}

fn reset_snapshot_for_process(
    snapshot: &mut RuntimeSnapshot,
    project: PathBuf,
    selected_session: Option<PathBuf>,
    status: String,
) {
    let auto_retry = snapshot.auto_retry;
    *snapshot = RuntimeSnapshot {
        status,
        project,
        selected_session,
        auto_retry,
        ..RuntimeSnapshot::default()
    };
}

fn failure_details(error: &str) -> String {
    let cleaned = error
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .collect::<String>();
    truncate_chars(cleaned.trim(), MAX_FAILURE_DETAILS_CHARS)
}

fn failure_summary(details: &str) -> String {
    let preferred = details.lines().rev().find_map(|line| {
        line.trim()
            .strip_prefix("Error:")
            .map(str::trim)
            .filter(|line| !line.is_empty())
    });
    let fallback = details.lines().rev().find_map(|line| {
        let line = line.trim();
        (!line.is_empty() && !line.starts_with("Warning:") && !line.starts_with("Hint:"))
            .then_some(line)
    });
    truncate_chars(
        preferred
            .or(fallback)
            .unwrap_or("Pi exited without an error message."),
        MAX_FAILURE_SUMMARY_CHARS,
    )
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut characters = value.chars();
    let mut truncated = characters.by_ref().take(max_chars).collect::<String>();
    if characters.next().is_some() {
        truncated.push('…');
    }
    truncated
}

fn startup_commands() -> [SessionCommand; 8] {
    [
        SessionCommand::ConfigureSteering,
        SessionCommand::LoadState,
        SessionCommand::LoadHistory,
        SessionCommand::LoadUsage,
        SessionCommand::ListModels,
        SessionCommand::ListReasoningLevels,
        SessionCommand::ListModes,
        SessionCommand::ListCommands,
    ]
}

const fn can_send_prompt(mode: PromptMode, running: bool, allow_while_running: bool) -> bool {
    allow_while_running || !running || !matches!(mode, PromptMode::Normal)
}

fn run_status(conversation: &ConversationState) -> &'static str {
    if conversation.compacting {
        "Compacting"
    } else if conversation.retrying {
        "Retrying"
    } else if conversation.running {
        "Working"
    } else if conversation.settled {
        "Ready"
    } else {
        "Idle"
    }
}

fn notification_target(snapshot: &RuntimeSnapshot) -> Option<(PathBuf, PathBuf)> {
    snapshot
        .live_session
        .clone()
        .or_else(|| snapshot.selected_session.clone())
        .map(|path| (path, snapshot.project.clone()))
}

fn session_badge_status(conversation: &ConversationState) -> &'static str {
    if conversation.compacting {
        "Compacting"
    } else if conversation.retrying {
        "Retrying"
    } else if conversation.running {
        "Working"
    } else {
        "Done"
    }
}

#[cfg(test)]
mod tests;
