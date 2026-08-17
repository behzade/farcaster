//! UI-neutral application runtime and active-session ownership.

mod catalog;
mod prompts;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::{
    agent_activity::AgentActivity,
    conversation::{ConversationState, TranscriptItem, TranscriptKind},
    protocol::{
        ExtensionUiResponse, Model, PromptImage, PromptMode, SessionState, SlashCommand, command,
    },
    rpc_process::{ProcessCommand, ProcessItem, RpcProcess},
    sessions::{SessionDiscovery, SessionSummary, discover, load_history},
    state::StateStore,
};

const MAX_IDLE_PI_ACTORS: usize = 4;
const ACTIVE_ROOT_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const ACTIVE_DESCENDANT_REFRESH_INTERVAL: Duration = Duration::from_secs(3);
const COALESCED_SESSION_REFRESH_DELAY: Duration = Duration::from_millis(100);

#[derive(Clone, Debug)]
pub(crate) enum RuntimeCommand {
    Prompt {
        target: String,
        mode: PromptMode,
        message: String,
        images: Vec<PromptImage>,
        allow_while_running: bool,
    },
    Abort,
    Reload,
    Compact {
        custom_instructions: Option<String>,
    },
    ExportHtml {
        output_path: Option<String>,
    },
    SetSessionName(String),
    NewSession {
        id: String,
        project: PathBuf,
    },
    ResumeDraft {
        id: String,
        project: PathBuf,
    },
    Resume {
        path: PathBuf,
        project: PathBuf,
    },
    SetModel {
        provider: String,
        model_id: String,
    },
    SetThinking(String),
    ExtensionResponse(ExtensionUiResponse),
    DeliverQueued(crate::state::QueuedPrompt),
    SetSettled {
        path: PathBuf,
        settled: bool,
    },
    LoadSessions(String),
    RefreshSessions,
    Shutdown,
}

#[derive(Clone, Debug)]
pub(crate) enum RuntimeEvent {
    Snapshot {
        generation: u64,
        snapshot: Arc<RuntimeSnapshot>,
    },
    SessionReset {
        generation: u64,
        preserve_submission: bool,
    },
    HistoryReset {
        generation: u64,
    },
    Sessions {
        generation: u64,
        sessions: Vec<SessionSummary>,
        all_sessions: Vec<SessionSummary>,
        activities: Option<(HashMap<String, AgentActivity>, bool)>,
        has_running_descendants: bool,
    },
    SessionsFailed {
        generation: u64,
        message: String,
    },
    RefreshCatalog,
    ExtensionUi {
        generation: u64,
        request: crate::protocol::ExtensionUiRequest,
    },
    PromptResult {
        generation: u64,
        target: String,
        accepted: bool,
        session: Option<PathBuf>,
    },
    SessionStatus {
        target: String,
        session: Option<PathBuf>,
        status: String,
    },
    Stopped,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RuntimeSnapshot {
    pub connected: bool,
    pub status: String,
    pub project: PathBuf,
    pub live_session: Option<PathBuf>,
    pub live_status: String,
    pub session: Option<SessionState>,
    pub prefill_model: Option<Model>,
    pub prefill_thinking_level: Option<String>,
    pub selected_session: Option<PathBuf>,
    pub conversation: ConversationState,
    pub models: Vec<Model>,
    pub thinking_levels: Vec<String>,
    pub stats: Value,
    pub commands: Vec<SlashCommand>,
    pub stderr: String,
    pub auto_retry: bool,
    pub history_preview: bool,
}

pub(crate) struct RuntimeHandle {
    commands: mpsc::Sender<RuntimeCommand>,
    events: mpsc::Receiver<RuntimeEvent>,
}

impl RuntimeHandle {
    pub(crate) fn spawn(
        project: PathBuf,
        draft_id: String,
        initial_session: Option<PathBuf>,
    ) -> Self {
        Self::spawn_with(
            project,
            draft_id,
            initial_session,
            ProcessCommand::default(),
        )
    }

    pub(crate) fn spawn_with(
        project: PathBuf,
        draft_id: String,
        initial_session: Option<PathBuf>,
        process_command: ProcessCommand,
    ) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        thread::Builder::new()
            .name("pi-gpui-supervisor".into())
            .spawn(move || {
                run_supervisor(
                    project,
                    draft_id,
                    initial_session,
                    process_command,
                    command_rx,
                    event_tx,
                );
            })
            .ok();
        Self { commands, events }
    }

    pub(crate) fn send(&self, command: RuntimeCommand) -> Result<(), String> {
        self.commands
            .send(command)
            .map_err(|_| "Pi runtime has stopped".to_owned())
    }

    pub(crate) fn try_recv(&self) -> Result<RuntimeEvent, mpsc::TryRecvError> {
        self.events.try_recv()
    }
}

use prompts::DeferredPrompt;

struct SessionRuntimeHandle {
    commands: mpsc::Sender<RuntimeCommand>,
    events: mpsc::Receiver<RuntimeEvent>,
}

impl SessionRuntimeHandle {
    fn spawn(project: PathBuf, process_command: ProcessCommand, load_catalog: bool) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        thread::Builder::new()
            .name("pi-gpui-session".into())
            .spawn(move || run(project, process_command, command_rx, event_tx, load_catalog))
            .ok();
        Self { commands, events }
    }

    fn send(&self, command: RuntimeCommand) {
        let _ = self.commands.send(command);
    }
}

fn run_supervisor(
    project: PathBuf,
    draft_id: String,
    initial_session: Option<PathBuf>,
    process_command: ProcessCommand,
    command_rx: mpsc::Receiver<RuntimeCommand>,
    event_tx: mpsc::Sender<RuntimeEvent>,
) {
    let initial_key = format!("draft:{draft_id}");
    let catalog_key = "catalog".to_owned();
    let initial_project = project.clone();
    let mut actors = HashMap::from([
        (
            catalog_key.clone(),
            SessionRuntimeHandle::spawn(project.clone(), process_command.clone(), true),
        ),
        (
            initial_key.clone(),
            SessionRuntimeHandle::spawn(project, process_command.clone(), false),
        ),
    ]);
    if let Some(actor) = actors.get(&initial_key) {
        actor.send(initial_draft_command(
            draft_id,
            initial_project,
            initial_session,
        ));
    }
    let mut selected = initial_key.clone();
    let mut generation = 0_u64;
    let mut latest = HashMap::<String, Arc<RuntimeSnapshot>>::new();
    let mut pending_extensions = HashMap::<String, Vec<crate::protocol::ExtensionUiRequest>>::new();
    let mut active_dialogs = HashMap::<String, Vec<crate::protocol::ExtensionUiRequest>>::new();
    let mut needs_input = HashSet::<String>::new();
    let mut clock = 0_u64;
    let mut last_touch = HashMap::from([(initial_key.clone(), clock)]);
    let mut catalog_has_running_descendants = false;
    let mut last_catalog_refresh = Instant::now();
    let mut session_controls = SessionControls::default();
    if let Ok(state) = StateStore::open()
        && let Ok(prompts) = state.queued_prompts()
    {
        for prompt in prompts {
            let key = prompt.target.clone();
            let actor = actors.entry(key).or_insert_with(|| {
                SessionRuntimeHandle::spawn(prompt.project.clone(), process_command.clone(), false)
            });
            actor.send(RuntimeCommand::DeliverQueued(prompt));
        }
    }
    let mut running = true;
    while running {
        let keys = actors.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            let mut events = Vec::new();
            if let Some(actor) = actors.get(&key) {
                while let Ok(event) = actor.events.try_recv() {
                    events.push(event);
                }
            }
            for event in events {
                clock = clock.saturating_add(1);
                last_touch.insert(key.clone(), clock);
                match event {
                    RuntimeEvent::Snapshot { snapshot, .. } => {
                        let mut snapshot = snapshot;
                        prefill_session_controls(
                            Arc::make_mut(&mut snapshot),
                            &mut session_controls,
                        );
                        if snapshot.conversation.settled {
                            needs_input.remove(&key);
                            active_dialogs.remove(&key);
                        }
                        let status = if needs_input.contains(&key) {
                            "Needs input"
                        } else {
                            semantic_status(&snapshot)
                        };
                        let _ = event_tx.send(RuntimeEvent::SessionStatus {
                            target: key.clone(),
                            session: snapshot
                                .live_session
                                .clone()
                                .or_else(|| snapshot.selected_session.clone()),
                            status: status.into(),
                        });
                        latest.insert(key.clone(), snapshot.clone());
                        if key == selected {
                            let _ = event_tx.send(RuntimeEvent::Snapshot {
                                generation,
                                snapshot,
                            });
                        }
                    }
                    RuntimeEvent::ExtensionUi { request, .. } => {
                        if request.dialog_id().is_some() {
                            active_dialogs
                                .entry(key.clone())
                                .or_default()
                                .push(request.clone());
                            needs_input.insert(key.clone());
                            let session = latest.get(&key).and_then(|snapshot| {
                                snapshot
                                    .live_session
                                    .clone()
                                    .or_else(|| snapshot.selected_session.clone())
                            });
                            let _ = event_tx.send(RuntimeEvent::SessionStatus {
                                target: key.clone(),
                                session,
                                status: "Needs input".into(),
                            });
                        }
                        if key == selected {
                            let _ = event_tx.send(RuntimeEvent::ExtensionUi {
                                generation,
                                request,
                            });
                        } else if request.dialog_id().is_none() {
                            pending_extensions
                                .entry(key.clone())
                                .or_default()
                                .push(request);
                        }
                    }
                    RuntimeEvent::SessionReset {
                        preserve_submission,
                        ..
                    } if key == selected => {
                        let _ = event_tx.send(RuntimeEvent::SessionReset {
                            generation,
                            preserve_submission,
                        });
                    }
                    RuntimeEvent::HistoryReset { .. } if key == selected => {
                        let _ = event_tx.send(RuntimeEvent::HistoryReset { generation });
                    }
                    RuntimeEvent::PromptResult {
                        target,
                        accepted,
                        session,
                        ..
                    } => {
                        let _ = event_tx.send(RuntimeEvent::PromptResult {
                            generation,
                            target,
                            accepted,
                            session,
                        });
                    }
                    RuntimeEvent::RefreshCatalog => {
                        if let Some(catalog) = actors.get(&catalog_key) {
                            catalog.send(RuntimeCommand::RefreshSessions);
                            last_catalog_refresh = Instant::now();
                        }
                    }
                    event @ (RuntimeEvent::Sessions { .. }
                    | RuntimeEvent::SessionsFailed { .. }) => {
                        match route_session_discovery(&key, &catalog_key, event) {
                            SupervisorSessionAction::Publish(event) => {
                                if let RuntimeEvent::Sessions {
                                    has_running_descendants,
                                    ..
                                } = &event
                                {
                                    catalog_has_running_descendants = *has_running_descendants;
                                }
                                let _ = event_tx.send(event);
                            }
                            SupervisorSessionAction::RefreshCatalog => {
                                if let Some(catalog) = actors.get(&catalog_key) {
                                    catalog.send(RuntimeCommand::RefreshSessions);
                                    last_catalog_refresh = Instant::now();
                                }
                            }
                        }
                    }
                    RuntimeEvent::Stopped
                    | RuntimeEvent::SessionStatus { .. }
                    | RuntimeEvent::SessionReset { .. }
                    | RuntimeEvent::HistoryReset { .. } => {}
                }
            }
        }
        evict_idle_actors(&mut actors, &mut latest, &mut last_touch, &selected);
        let has_running_root = latest
            .values()
            .any(|snapshot| snapshot.conversation.running);
        let refresh_interval = if has_running_root {
            Some(ACTIVE_ROOT_REFRESH_INTERVAL)
        } else if catalog_has_running_descendants {
            Some(ACTIVE_DESCENDANT_REFRESH_INTERVAL)
        } else {
            None
        };
        if refresh_interval.is_some_and(|interval| last_catalog_refresh.elapsed() >= interval)
            && let Some(catalog) = actors.get(&catalog_key)
        {
            catalog.send(RuntimeCommand::RefreshSessions);
            last_catalog_refresh = Instant::now();
        }
        match command_rx.recv_timeout(Duration::from_millis(12)) {
            Ok(RuntimeCommand::Shutdown) => running = false,
            Ok(command) => {
                if matches!(&command, RuntimeCommand::ExtensionResponse(_)) {
                    if let Some(dialogs) = active_dialogs.get_mut(&selected) {
                        if !dialogs.is_empty() {
                            dialogs.remove(0);
                        }
                        if dialogs.is_empty() {
                            active_dialogs.remove(&selected);
                            needs_input.remove(&selected);
                        }
                    }
                    let session = latest.get(&selected).and_then(|snapshot| {
                        snapshot
                            .live_session
                            .clone()
                            .or_else(|| snapshot.selected_session.clone())
                    });
                    let _ = event_tx.send(RuntimeEvent::SessionStatus {
                        target: selected.clone(),
                        session,
                        status: "Working".into(),
                    });
                }
                let next = command_target(&command);
                if let Some((requested_key, project)) = next {
                    let key = actor_key_for_command(&command, &requested_key, &latest);
                    clock = clock.saturating_add(1);
                    last_touch.insert(key.clone(), clock);
                    let selection_changed = key != selected;
                    if selection_changed {
                        generation = generation.saturating_add(1);
                        selected = key.clone();
                        let _ = event_tx.send(RuntimeEvent::SessionReset {
                            generation,
                            preserve_submission: false,
                        });
                    }
                    let actor = actors.entry(key.clone()).or_insert_with(|| {
                        SessionRuntimeHandle::spawn(project, process_command.clone(), false)
                    });
                    actor.send(command);
                    if let Some(snapshot) = latest.get(&key).cloned() {
                        let _ = event_tx.send(RuntimeEvent::Snapshot {
                            generation,
                            snapshot,
                        });
                    }
                    if let Some(requests) = pending_extensions.remove(&key) {
                        for request in requests {
                            let _ = event_tx.send(RuntimeEvent::ExtensionUi {
                                generation,
                                request,
                            });
                        }
                    }
                    if selection_changed && let Some(dialogs) = active_dialogs.get(&key) {
                        for request in dialogs {
                            let _ = event_tx.send(RuntimeEvent::ExtensionUi {
                                generation,
                                request: request.clone(),
                            });
                        }
                    }
                } else {
                    let target = if matches!(
                        &command,
                        RuntimeCommand::LoadSessions(_)
                            | RuntimeCommand::RefreshSessions
                            | RuntimeCommand::SetSettled { .. }
                    ) {
                        &catalog_key
                    } else {
                        &selected
                    };
                    if let Some(actor) = actors.get(target) {
                        actor.send(command);
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => running = false,
        }
    }
    for actor in actors.values() {
        actor.send(RuntimeCommand::Shutdown);
    }
    let _ = event_tx.send(RuntimeEvent::Stopped);
}

fn initial_draft_command(id: String, project: PathBuf, session: Option<PathBuf>) -> RuntimeCommand {
    session.map_or(
        RuntimeCommand::ResumeDraft {
            id,
            project: project.clone(),
        },
        |path| RuntimeCommand::Resume { path, project },
    )
}

#[derive(Debug)]
enum SupervisorSessionAction {
    Publish(RuntimeEvent),
    RefreshCatalog,
}

fn route_session_discovery(
    actor_key: &str,
    catalog_key: &str,
    event: RuntimeEvent,
) -> SupervisorSessionAction {
    if actor_key == catalog_key {
        SupervisorSessionAction::Publish(event)
    } else {
        SupervisorSessionAction::RefreshCatalog
    }
}

fn command_target(command: &RuntimeCommand) -> Option<(String, PathBuf)> {
    match command {
        RuntimeCommand::NewSession { id, project }
        | RuntimeCommand::ResumeDraft { id, project } => {
            Some((format!("draft:{id}"), project.clone()))
        }
        RuntimeCommand::Resume { path, project } => {
            Some((format!("session:{}", path.display()), project.clone()))
        }
        _ => None,
    }
}

fn actor_key_for_command(
    command: &RuntimeCommand,
    requested_key: &str,
    latest: &HashMap<String, Arc<RuntimeSnapshot>>,
) -> String {
    let RuntimeCommand::Resume { path, .. } = command else {
        return requested_key.to_owned();
    };
    latest
        .iter()
        .find(|(_, snapshot)| {
            snapshot.live_session.as_deref() == Some(path.as_path())
                || snapshot.selected_session.as_deref() == Some(path.as_path())
        })
        .map_or_else(|| requested_key.to_owned(), |(key, _)| key.clone())
}

#[derive(Default)]
struct SessionControls {
    model: Option<Model>,
    thinking_level: Option<String>,
    models: Vec<Model>,
    thinking_levels: Vec<String>,
}

fn prefill_session_controls(snapshot: &mut RuntimeSnapshot, controls: &mut SessionControls) {
    if let Some(session) = &snapshot.session {
        if let Some(model) = &session.model {
            controls.model = Some(model.clone());
        }
        controls.thinking_level = Some(session.thinking_level.clone());
    } else {
        snapshot.prefill_model = controls.model.clone();
        snapshot.prefill_thinking_level = controls.thinking_level.clone();
    }
    if snapshot.models.is_empty() {
        snapshot.models.clone_from(&controls.models);
    } else {
        controls.models.clone_from(&snapshot.models);
    }
    if snapshot.thinking_levels.is_empty() {
        snapshot
            .thinking_levels
            .clone_from(&controls.thinking_levels);
    } else {
        controls
            .thinking_levels
            .clone_from(&snapshot.thinking_levels);
    }
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

fn evict_idle_actors(
    actors: &mut HashMap<String, SessionRuntimeHandle>,
    latest: &mut HashMap<String, Arc<RuntimeSnapshot>>,
    last_touch: &mut HashMap<String, u64>,
    selected: &str,
) {
    let connected = latest
        .iter()
        .filter(|(_, snapshot)| snapshot.connected)
        .count();
    if connected <= MAX_IDLE_PI_ACTORS {
        return;
    }
    let mut candidates = latest
        .iter()
        .filter(|(key, snapshot)| {
            key.as_str() != selected && snapshot.connected && semantic_status(snapshot) == "Done"
        })
        .map(|(key, _)| {
            (
                last_touch.get(key).copied().unwrap_or_default(),
                key.clone(),
            )
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(touch, _)| *touch);
    for (_, key) in candidates
        .into_iter()
        .take(connected.saturating_sub(MAX_IDLE_PI_ACTORS))
    {
        if let Some(actor) = actors.remove(&key) {
            actor.send(RuntimeCommand::Shutdown);
        }
        latest.remove(&key);
        last_touch.remove(&key);
    }
}

struct RuntimeOwner {
    project: PathBuf,
    process_command: ProcessCommand,
    process: Option<RpcProcess>,
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
    event_tx: mpsc::Sender<RuntimeEvent>,
    discovery_tx: mpsc::Sender<DiscoveryResult>,
    history_tx: mpsc::Sender<HistoryResult>,
    history_generation: u64,
    active_session: Option<PathBuf>,
    parked_snapshot: Option<RuntimeSnapshot>,
    deferred_prompt: Option<DeferredPrompt>,
    startup_state_loaded: bool,
    startup_history_loaded: bool,
    state: Option<StateStore>,
    session_query: String,
}

struct DiscoveryResult {
    generation: u64,
    result: Result<SessionDiscovery, String>,
}

struct HistoryResult {
    generation: u64,
    path: PathBuf,
    project: PathBuf,
    result: Result<Vec<Value>, String>,
}

fn run(
    project: PathBuf,
    process_command: ProcessCommand,
    command_rx: mpsc::Receiver<RuntimeCommand>,
    event_tx: mpsc::Sender<RuntimeEvent>,
    load_catalog: bool,
) {
    let (discovery_tx, discovery_rx) = mpsc::channel();
    let (history_tx, history_rx) = mpsc::channel();
    let (state, state_error) = match StateStore::open() {
        Ok(state) => (Some(state), None),
        Err(error) => (None, Some(error)),
    };
    let mut owner = RuntimeOwner {
        project: project.clone(),
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
        event_tx,
        discovery_tx,
        history_tx,
        history_generation: 0,
        active_session: None,
        parked_snapshot: None,
        deferred_prompt: None,
        startup_state_loaded: false,
        startup_history_loaded: false,
        state,
        session_query: String::new(),
    };
    if let Some(error) = state_error {
        owner
            .snapshot
            .conversation
            .push_local_error("State unavailable", error);
    }
    if load_catalog {
        owner.load_sessions(String::new());
    }
    owner.publish();
    let mut running = true;
    while running {
        while let Ok(result) = discovery_rx.try_recv() {
            owner.apply_discovery(result);
        }
        while let Ok(result) = history_rx.try_recv() {
            owner.apply_history(result);
        }
        owner.poll_deferred_session_refresh(Instant::now());
        let mut process_snapshot_changed = false;
        while let Some(item) = owner.process.as_mut().and_then(RpcProcess::try_next) {
            process_snapshot_changed |= owner.apply_process_item(item);
        }
        if process_snapshot_changed {
            owner.publish();
        }
        match command_rx.recv_timeout(Duration::from_millis(12)) {
            Ok(RuntimeCommand::Shutdown) => running = false,
            Ok(command) => owner.apply_command(command),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => running = false,
        }
    }
    if let Some(mut process) = owner.process.take() {
        let _ = process.terminate();
    }
    let _ = owner.event_tx.send(RuntimeEvent::Stopped);
}

impl RuntimeOwner {
    fn start_process(&mut self, session: Option<PathBuf>) {
        self.process_generation = self.process_generation.saturating_add(1);
        let preserve_submission = self.deferred_prompt.is_some();
        let keep_preview = preserve_submission && self.snapshot.history_preview;
        if let Some(mut old) = self.process.take() {
            let _ = old.terminate();
        }
        self.active_session = session.clone();
        self.parked_snapshot = None;
        self.startup_state_loaded = false;
        self.startup_history_loaded = false;
        self.pending_prompt_id = None;
        self.pending_prompt_item = None;
        let status = session.as_ref().map_or_else(
            || "Starting new session".into(),
            |_| "Resuming session".into(),
        );
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
        }
        let _ = self.event_tx.send(RuntimeEvent::SessionReset {
            generation: self.process_generation,
            preserve_submission,
        });
        self.publish();
        match RpcProcess::spawn(&self.process_command, &self.project, session.as_deref()) {
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
        for kind in [
            "get_state",
            "get_messages",
            "get_session_stats",
            "get_available_models",
            "get_available_thinking_levels",
            "get_commands",
        ] {
            self.send(command(kind));
        }
    }

    fn apply_command(&mut self, runtime_command: RuntimeCommand) {
        match runtime_command {
            RuntimeCommand::Prompt {
                target,
                mode,
                message,
                images,
                allow_while_running,
            } => self.send_prompt(target, mode, message, images, allow_while_running),
            RuntimeCommand::DeliverQueued(prompt) => self.deliver_queued(prompt),
            RuntimeCommand::Abort => self.send(command("abort")),
            RuntimeCommand::Reload => self.reload(),
            RuntimeCommand::Compact {
                custom_instructions,
            } => self.send(optional_string_command(
                "compact",
                "customInstructions",
                custom_instructions,
            )),
            RuntimeCommand::ExportHtml { output_path } => self.send(optional_string_command(
                "export_html",
                "outputPath",
                output_path,
            )),
            RuntimeCommand::SetSessionName(name) => {
                self.send(json!({"type": "set_session_name", "name": name}))
            }
            RuntimeCommand::NewSession { project, .. } => {
                self.project = project;
                self.start_process(None);
            }
            RuntimeCommand::ResumeDraft { project, .. } => self.resume_draft(project),
            RuntimeCommand::Resume { path, project } => self.preview_history(path, project),
            RuntimeCommand::SetModel { provider, model_id } => {
                self.send(json!({"type":"set_model","provider":provider,"modelId":model_id}))
            }
            RuntimeCommand::SetThinking(level) => {
                self.send(json!({"type":"set_thinking_level","level":level}))
            }
            RuntimeCommand::ExtensionResponse(response) => {
                if let Some(process) = self.process.as_mut()
                    && let Err(error) = process.send_extension_response(response)
                {
                    self.fail(error);
                }
            }
            RuntimeCommand::SetSettled { path, settled } => {
                if let Some(state) = &self.state
                    && let Err(error) = state.set_settled(&path, settled)
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
            snapshot.conversation.push_local_error(
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

    fn send(&mut self, command: Value) {
        let command_name = command
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("command")
            .to_owned();
        match self
            .process
            .as_mut()
            .map(|process| process.send_command(command))
        {
            Some(Ok(_)) => {}
            Some(Err(error)) => self.fail(error),
            None => self.fail(format!("Cannot send {command_name}: Pi is not connected")),
        }
    }

    fn apply_process_item(&mut self, item: ProcessItem) -> bool {
        match item {
            ProcessItem::Response(response) => {
                self.apply_response(response);
                false
            }
            ProcessItem::ExtensionUi(request) => {
                let _ = self.event_tx.send(RuntimeEvent::ExtensionUi {
                    generation: self.process_generation,
                    request,
                });
                false
            }
            ProcessItem::Event(event) => {
                let settled = event.get("type").and_then(Value::as_str) == Some("agent_settled");
                let previewing = self.parked_snapshot.is_some();
                let previous_live_status =
                    previewing.then(|| session_badge_status(&self.active_snapshot().conversation));
                let snapshot = self.active_snapshot_mut();
                snapshot.conversation.reduce(&event);
                snapshot.status = run_status(&snapshot.conversation).to_owned();
                let live_status_changed = previous_live_status
                    .is_some_and(|status| status != session_badge_status(&snapshot.conversation));
                let should_publish = !previewing || live_status_changed;
                if settled {
                    self.send(command("get_state"));
                    self.send(command("get_session_stats"));
                    self.refresh_sessions();
                }
                should_publish
            }
            ProcessItem::Stderr(chunk) => {
                let previewing = self.parked_snapshot.is_some();
                let snapshot = self.active_snapshot_mut();
                snapshot.stderr.push_str(&chunk);
                if snapshot.stderr.len() > 32 * 1024 {
                    snapshot.stderr.drain(..16 * 1024);
                }
                !previewing
            }
            ProcessItem::Failure(error) => {
                self.fail(error);
                false
            }
        }
    }

    fn active_snapshot_mut(&mut self) -> &mut RuntimeSnapshot {
        self.parked_snapshot.as_mut().unwrap_or(&mut self.snapshot)
    }

    fn active_snapshot(&self) -> &RuntimeSnapshot {
        self.parked_snapshot.as_ref().unwrap_or(&self.snapshot)
    }

    fn preview_history(&mut self, path: PathBuf, project: PathBuf) {
        self.history_generation = self.history_generation.saturating_add(1);
        if self.snapshot.selected_session.as_deref() == Some(path.as_path()) {
            return;
        }
        if self.active_session.as_deref() == Some(path.as_path()) {
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
        let generation = self.history_generation;
        let sender = self.history_tx.clone();
        thread::Builder::new()
            .name("pi-gpui-history".into())
            .spawn(move || {
                let result = load_history(&path);
                let _ = sender.send(HistoryResult {
                    generation,
                    path,
                    project,
                    result,
                });
            })
            .ok();
    }

    fn resume_draft(&mut self, project: PathBuf) {
        let already_active = self.process.is_some()
            && self.parked_snapshot.is_none()
            && !self.snapshot.history_preview
            && self.project == project;
        if already_active {
            self.publish();
            return;
        }
        let can_restore = self.active_session.is_none()
            && self
                .parked_snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.project == project);
        if can_restore && let Some(snapshot) = self.parked_snapshot.take() {
            self.snapshot = snapshot;
            self.project = project;
            let _ = self.event_tx.send(RuntimeEvent::HistoryReset {
                generation: self.process_generation,
            });
            self.publish();
            return;
        }
        self.project = project;
        self.start_process(None);
    }

    fn apply_history(&mut self, result: HistoryResult) {
        if result.generation != self.history_generation {
            return;
        }
        let messages = match result.result {
            Ok(messages) => messages,
            Err(error) => {
                self.snapshot.status = "Could not load history".into();
                self.snapshot
                    .conversation
                    .push_local_error("History unavailable", error);
                self.publish();
                return;
            }
        };
        if self.parked_snapshot.is_none() {
            self.parked_snapshot = Some(std::mem::take(&mut self.snapshot));
        }
        self.project = result.project.clone();
        let auto_retry = self
            .parked_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.auto_retry);
        let mut conversation = ConversationState::default();
        conversation.replace_history(&messages);
        self.snapshot = RuntimeSnapshot {
            connected: true,
            status: "Ready".into(),
            project: result.project,
            selected_session: Some(result.path),
            conversation,
            auto_retry,
            history_preview: true,
            ..RuntimeSnapshot::default()
        };
        let _ = self.event_tx.send(RuntimeEvent::HistoryReset {
            generation: self.process_generation,
        });
        self.publish();
    }

    fn apply_response(&mut self, response: crate::protocol::RpcResponse) {
        let response_command = response.command.clone();
        let is_prompt_response =
            matches!(response.command.as_str(), "prompt" | "steer" | "follow_up")
                && response.id.as_ref() == self.pending_prompt_id.as_ref();
        if is_prompt_response {
            self.pending_prompt_id = None;
            if response.success {
                let target = self.pending_prompt_target.clone().unwrap_or_default();
                let session = self.active_session.clone();
                if let Some(id) = self.pending_outbox_id.take()
                    && let Some(state) = self.state.as_mut()
                {
                    let _ = state.complete_prompt(id, &target, session.as_deref());
                }
            } else {
                self.mark_outbox_failed(
                    response
                        .error
                        .as_deref()
                        .unwrap_or("Pi rejected the prompt"),
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
            let blocks_resume = self.deferred_prompt.is_some()
                && matches!(response.command.as_str(), "get_state" | "get_messages");
            let snapshot = self.active_snapshot_mut();
            snapshot.conversation.push_local_error(
                "Command failed",
                format!(
                    "{}: {}",
                    response.command,
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
        match response.command.as_str() {
            "get_state" => match serde_json::from_value::<SessionState>(response.data) {
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
                    snapshot.conversation.running = state.is_streaming;
                    snapshot.session = Some(state);
                    snapshot.status = "Ready".into();
                    self.startup_state_loaded = true;
                    if self.active_session.is_some() && self.active_session != previous_session {
                        self.refresh_sessions();
                    }
                }
                Err(error) => {
                    self.fail(format!("decode get_state: {error}"));
                    return;
                }
            },
            "get_messages" => {
                let messages = response
                    .data
                    .get("messages")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                self.active_snapshot_mut()
                    .conversation
                    .replace_history(&messages);
                self.startup_history_loaded = true;
            }
            "get_available_models" => {
                self.active_snapshot_mut().models = response
                    .data
                    .get("models")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default();
            }
            "get_available_thinking_levels" => {
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
            "get_session_stats" => self.active_snapshot_mut().stats = response.data,
            "get_commands" => {
                self.active_snapshot_mut().commands = response
                    .data
                    .get("commands")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|command| serde_json::from_value(command.clone()).ok())
                    .collect()
            }
            "set_model" => {
                if let Ok(model) = serde_json::from_value::<Model>(response.data)
                    && let Some(state) = self.active_snapshot_mut().session.as_mut()
                {
                    state.model = Some(model);
                }
                self.send(command("get_available_thinking_levels"));
                self.send(command("get_state"));
            }
            "set_thinking_level" => {
                self.send(command("get_state"));
            }
            "new_session"
                if response.data.get("cancelled").and_then(Value::as_bool) != Some(true) =>
            {
                self.process_generation = self.process_generation.saturating_add(1);
                self.active_session = None;
                self.pending_prompt_id = None;
                reset_snapshot_for_live_session(
                    self.active_snapshot_mut(),
                    "Loading new session".into(),
                );
                let _ = self.event_tx.send(RuntimeEvent::SessionReset {
                    generation: self.process_generation,
                    preserve_submission: false,
                });
                self.publish();
                self.send_startup_queries();
                self.refresh_sessions();
            }
            "prompt" | "steer" | "follow_up" => {
                self.active_snapshot_mut().status = "Accepted".into();
                self.send(command("get_state"));
            }
            "abort" => self.active_snapshot_mut().status = "Stopping".into(),
            "compact" | "set_auto_compaction" | "set_auto_retry" | "abort_retry" => {
                self.send(command("get_state"))
            }
            "set_session_name" => {
                self.active_snapshot_mut().status = "Session named".into();
                self.send(command("get_state"));
                self.refresh_sessions();
            }
            "export_html" => {
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
        if matches!(response_command.as_str(), "get_state" | "get_messages") {
            self.maybe_send_deferred_prompt();
        }
        if self.parked_snapshot.is_none() {
            self.publish();
        }
    }

    fn rollback_pending_prompt(&mut self) {
        if let Some(optimistic) = self.pending_prompt_item.take() {
            self.active_snapshot_mut()
                .conversation
                .rollback_local_user(&optimistic);
        }
    }

    fn fail(&mut self, error: String) {
        self.pending_prompt_id = None;
        self.deferred_prompt = None;
        self.rollback_pending_prompt();
        if let Some(target) = self.pending_prompt_target.take() {
            self.emit_prompt_result(&target, false);
        }
        if let Some(mut process) = self.process.take() {
            let _ = process.terminate();
        }
        let previewing = self.parked_snapshot.is_some();
        let snapshot = self.active_snapshot_mut();
        snapshot.connected = false;
        snapshot.status = "Failed".into();
        snapshot.conversation.diagnostics.push(error);
        snapshot
            .conversation
            .push_local_error("Couldn’t send", "Try again from the composer.".into());
        if previewing && let Some(snapshot) = self.parked_snapshot.take() {
            self.snapshot = snapshot;
        }
        self.publish();
    }

    fn mark_outbox_failed(&mut self, error: &str) {
        if let Some(id) = self.pending_outbox_id.take()
            && let Some(state) = &self.state
        {
            let _ = state.fail_prompt(id, error);
        }
    }

    fn publish(&self) {
        let active_snapshot = self.active_snapshot();
        let mut snapshot = self.snapshot.clone();
        snapshot.live_session = self
            .active_session
            .clone()
            .or_else(|| active_snapshot.selected_session.clone());
        snapshot.live_status = session_badge_status(&active_snapshot.conversation).into();
        let _ = self.event_tx.send(RuntimeEvent::Snapshot {
            generation: self.process_generation,
            snapshot: Arc::new(snapshot),
        });
    }
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

fn reset_snapshot_for_live_session(snapshot: &mut RuntimeSnapshot, status: String) {
    let auto_retry = snapshot.auto_retry;
    let project = snapshot.project.clone();
    *snapshot = RuntimeSnapshot {
        connected: true,
        status,
        project,
        auto_retry,
        ..RuntimeSnapshot::default()
    };
}

fn optional_string_command(kind: &str, field: &str, value: Option<String>) -> Value {
    let mut command = serde_json::Map::from_iter([("type".into(), Value::String(kind.into()))]);
    if let Some(value) = value {
        command.insert(field.into(), Value::String(value));
    }
    Value::Object(command)
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

fn has_running_descendant(sessions: &[SessionSummary]) -> bool {
    sessions
        .iter()
        .any(|session| session.parent_session.is_some() && session.is_running)
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
mod tests {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt as _, symlink},
        time::{Duration, Instant, SystemTime},
    };

    use tempfile::tempdir;

    use super::*;

    fn owner_without_process(
        project: PathBuf,
    ) -> (
        RuntimeOwner,
        mpsc::Receiver<RuntimeEvent>,
        mpsc::Receiver<DiscoveryResult>,
    ) {
        let (event_tx, event_rx) = mpsc::channel();
        let (discovery_tx, discovery_rx) = mpsc::channel();
        let (history_tx, _history_rx) = mpsc::channel();
        (
            RuntimeOwner {
                project: project.clone(),
                process_command: ProcessCommand {
                    program: PathBuf::from("/definitely/missing/pi-gpui-test-command"),
                    prefix_args: Vec::new(),
                    direnv_program: None,
                },
                process: None,
                snapshot: RuntimeSnapshot {
                    connected: true,
                    status: "Ready".into(),
                    project,
                    ..RuntimeSnapshot::default()
                },
                owns_session_catalog: true,
                session_generation: 0,
                session_discovery_in_flight: false,
                session_refresh_pending: false,
                session_refresh_due: None,
                process_generation: 1,
                pending_prompt_id: None,
                pending_prompt_target: None,
                pending_prompt_item: None,
                pending_outbox_id: None,
                event_tx,
                discovery_tx,
                history_tx,
                history_generation: 0,
                active_session: None,
                parked_snapshot: None,
                deferred_prompt: None,
                startup_state_loaded: false,
                startup_history_loaded: false,
                state: None,
                session_query: String::new(),
            },
            event_rx,
            discovery_rx,
        )
    }

    #[test]
    fn persisted_submitted_draft_resumes_its_session() {
        let project = PathBuf::from("/project");
        let session = PathBuf::from("/sessions/submitted.jsonl");
        assert!(matches!(
            initial_draft_command("draft".into(), project.clone(), Some(session.clone())),
            RuntimeCommand::Resume { path, project: resumed_project }
                if path == session && resumed_project == project
        ));
        assert!(matches!(
            initial_draft_command("draft".into(), project.clone(), None),
            RuntimeCommand::ResumeDraft { id, project: draft_project }
                if id == "draft" && draft_project == project
        ));
    }

    #[test]
    fn snapshot_event_clones_share_transcript_storage() {
        let event = RuntimeEvent::Snapshot {
            generation: 1,
            snapshot: Arc::new(RuntimeSnapshot::default()),
        };
        let cloned = event.clone();
        let RuntimeEvent::Snapshot { snapshot: left, .. } = &event else {
            panic!("expected snapshot event");
        };
        let RuntimeEvent::Snapshot {
            snapshot: right, ..
        } = &cloned
        else {
            panic!("expected cloned snapshot event");
        };

        assert!(Arc::ptr_eq(left, right));
    }

    #[test]
    fn failed_tool_does_not_mark_the_whole_session_failed() {
        let mut conversation = ConversationState::default();
        conversation.replace_history(&[
            json!({"role":"assistant","content":[{
                "type":"toolCall","id":"read-1","name":"read","arguments":{"path":"x"}
            }]}),
            json!({
                "role":"toolResult","toolCallId":"read-1","toolName":"read",
                "content":[{"type":"text","text":"not found"}],"isError":true
            }),
        ]);
        assert!(conversation.items[0].is_error);
        assert_eq!(conversation.items[0].kind, TranscriptKind::Tool);
        assert_eq!(
            semantic_status(&RuntimeSnapshot {
                conversation,
                ..RuntimeSnapshot::default()
            }),
            "Done"
        );
    }

    #[test]
    fn history_preview_does_not_claim_to_know_an_external_run_failed() {
        let mut conversation = ConversationState::default();
        conversation.push_local_error("Previous error", "stale".into());
        assert_eq!(
            semantic_status(&RuntimeSnapshot {
                selected_session: Some(PathBuf::from("session.jsonl")),
                conversation,
                history_preview: true,
                ..RuntimeSnapshot::default()
            }),
            "Done"
        );
    }

    #[test]
    fn agent_end_does_not_report_ready_until_settled() {
        let mut conversation = ConversationState::default();
        conversation.reduce(&json!({"type":"agent_start"}));
        conversation.reduce(&json!({"type":"agent_end","willRetry":false}));
        assert_eq!(run_status(&conversation), "Working");
        conversation.reduce(&json!({"type":"agent_settled"}));
        assert_eq!(run_status(&conversation), "Ready");
    }

    #[test]
    fn saved_path_reuses_the_actor_that_started_as_a_draft() {
        let path = PathBuf::from("/sessions/one.jsonl");
        let command = RuntimeCommand::Resume {
            path: path.clone(),
            project: PathBuf::from("/project"),
        };
        let latest = HashMap::from([(
            "draft:one".into(),
            Arc::new(RuntimeSnapshot {
                live_session: Some(path.clone()),
                ..RuntimeSnapshot::default()
            }),
        )]);

        assert_eq!(
            actor_key_for_command(&command, &format!("session:{}", path.display()), &latest,),
            "draft:one"
        );
    }

    #[test]
    fn non_catalog_discovery_refreshes_catalog_without_publishing_local_results() {
        let action = route_session_discovery(
            "draft:a",
            "catalog",
            RuntimeEvent::Sessions {
                generation: 99,
                sessions: Vec::new(),
                all_sessions: Vec::new(),
                activities: None,
                has_running_descendants: false,
            },
        );

        assert!(matches!(action, SupervisorSessionAction::RefreshCatalog));
    }

    #[test]
    fn catalog_discovery_is_the_authoritative_generation_namespace() {
        let action = route_session_discovery(
            "catalog",
            "catalog",
            RuntimeEvent::Sessions {
                generation: 7,
                sessions: Vec::new(),
                all_sessions: Vec::new(),
                activities: None,
                has_running_descendants: false,
            },
        );

        assert!(matches!(
            action,
            SupervisorSessionAction::Publish(RuntimeEvent::Sessions { generation: 7, .. })
        ));
    }

    #[test]
    fn non_catalog_discovery_failures_also_refresh_instead_of_publishing() {
        let action = route_session_discovery(
            "session:/one",
            "catalog",
            RuntimeEvent::SessionsFailed {
                generation: 41,
                message: "local failure".into(),
            },
        );

        assert!(matches!(action, SupervisorSessionAction::RefreshCatalog));
    }

    #[test]
    fn non_catalog_actors_request_one_authoritative_refresh_without_scanning() {
        let (mut owner, events, _discovery) = owner_without_process(std::env::temp_dir());
        owner.owns_session_catalog = false;

        owner.refresh_sessions();

        assert_eq!(owner.session_generation, 0);
        assert!(matches!(
            events.try_recv(),
            Ok(RuntimeEvent::RefreshCatalog)
        ));
    }

    #[test]
    fn optional_builtin_rpc_arguments_are_omitted_when_absent() {
        assert_eq!(
            optional_string_command("compact", "customInstructions", None),
            json!({"type":"compact"})
        );
        assert_eq!(
            optional_string_command(
                "compact",
                "customInstructions",
                Some("focus on code".into()),
            ),
            json!({"type":"compact","customInstructions":"focus on code"})
        );
        assert_eq!(
            optional_string_command("export_html", "outputPath", Some("out.html".into())),
            json!({"type":"export_html","outputPath":"out.html"})
        );
    }

    #[test]
    fn streaming_accepts_queued_messages_and_exact_extension_commands() {
        assert!(!can_send_prompt(PromptMode::Normal, true, false));
        assert!(can_send_prompt(PromptMode::Steer, true, false));
        assert!(can_send_prompt(PromptMode::FollowUp, true, false));
        assert!(can_send_prompt(PromptMode::Normal, false, false));
        assert!(can_send_prompt(PromptMode::Normal, true, true));
    }

    #[test]
    fn reload_is_rejected_while_the_session_is_running() {
        let (mut owner, _events, _discovery) = owner_without_process(std::env::temp_dir());
        owner
            .snapshot
            .conversation
            .reduce(&json!({"type":"agent_start"}));
        let generation = owner.process_generation;

        owner.apply_command(RuntimeCommand::Reload);

        assert_eq!(owner.process_generation, generation);
        assert_eq!(owner.snapshot.status, "Reload not started");
        assert!(
            owner
                .snapshot
                .conversation
                .items
                .iter()
                .any(|item| item.text.contains("Wait for the current response"))
        );
    }

    #[test]
    fn reload_restarts_the_idle_session_process() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let script = temp.path().join("fake-pi.sh");
        fs::write(&script, include_str!("../tests/fixtures/fake-pi.sh"))?;
        let mut permissions = fs::metadata(&script)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions)?;
        let session = temp.path().join("session.jsonl");
        let (mut owner, _events, _discovery) = owner_without_process(temp.path().to_path_buf());
        owner.process_command = ProcessCommand {
            program: script,
            prefix_args: vec!["quiet".into()],
            direnv_program: None,
        };
        owner.active_session = Some(session.clone());
        owner.snapshot.selected_session = Some(session.clone());
        let generation = owner.process_generation;

        owner.apply_command(RuntimeCommand::Reload);

        assert!(owner.process.is_some());
        assert_eq!(owner.process_generation, generation + 1);
        assert_eq!(owner.snapshot.selected_session, Some(session));
        assert!(owner.snapshot.connected);
        if let Some(mut process) = owner.process.take() {
            process.terminate()?;
        }
        Ok(())
    }

    #[test]
    fn deferred_prompt_is_rejected_when_startup_state_has_no_session_path()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let script = temp.path().join("fake-pi.sh");
        fs::write(&script, include_str!("../tests/fixtures/fake-pi.sh"))?;
        let mut permissions = fs::metadata(&script)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions)?;
        let process = RpcProcess::spawn(
            &ProcessCommand {
                program: script,
                prefix_args: vec!["quiet".into()],
                direnv_program: None,
            },
            temp.path(),
            None,
        )?;
        let (mut owner, events, _discovery) = owner_without_process(temp.path().to_path_buf());
        owner.process = Some(process);
        owner.state = Some(StateStore::open_at(&temp.path().join("gui-state.sqlite3"))?);

        owner.send_prompt(
            "draft:a".into(),
            PromptMode::Normal,
            "hello".into(),
            Vec::new(),
            false,
        );

        assert!(owner.deferred_prompt.is_some());
        assert!(
            events
                .try_iter()
                .all(|event| !matches!(event, RuntimeEvent::PromptResult { .. }))
        );

        owner.startup_state_loaded = true;
        owner.startup_history_loaded = true;
        owner.maybe_send_deferred_prompt();

        assert!(owner.deferred_prompt.is_none());
        assert!(owner.pending_prompt_item.is_none());
        assert!(!owner.snapshot.conversation.running);
        assert!(
            owner
                .state
                .as_ref()
                .expect("state")
                .queued_prompts()?
                .is_empty()
        );
        assert!(events.try_iter().any(|event| matches!(
            event,
            RuntimeEvent::PromptResult {
                target,
                accepted: false,
                session: None,
                ..
            } if target == "draft:a"
        )));
        Ok(())
    }

    #[test]
    fn accepted_prompt_result_has_the_normalized_active_session_path()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let session = temp.path().join("session.jsonl");
        let link = temp.path().join("link.jsonl");
        fs::write(&session, "{}")?;
        symlink(&session, &link)?;
        let (mut owner, events, _discovery) = owner_without_process(temp.path().to_path_buf());
        owner.active_session = Some(crate::sessions::normalize_session_path(&link));

        owner.emit_prompt_result("draft:a", true);

        assert!(events.try_iter().any(|event| matches!(
            event,
            RuntimeEvent::PromptResult {
                accepted: true,
                session: Some(path),
                ..
            } if path == session.canonicalize().expect("canonical session")
        )));
        Ok(())
    }

    #[test]
    fn new_session_starts_pi_before_the_first_prompt() -> Result<(), Box<dyn std::error::Error>> {
        let old_project = tempdir()?;
        let new_project = tempdir()?;
        let script = old_project.path().join("fake-pi.sh");
        fs::write(&script, include_str!("../tests/fixtures/fake-pi.sh"))?;
        let mut permissions = fs::metadata(&script)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions)?;
        let (mut owner, _events, _discovery) =
            owner_without_process(old_project.path().to_path_buf());
        owner.process_command = ProcessCommand {
            program: script,
            prefix_args: vec!["quiet".into()],
            direnv_program: None,
        };

        owner.apply_command(RuntimeCommand::NewSession {
            id: "draft-new".into(),
            project: new_project.path().to_path_buf(),
        });

        assert_eq!(owner.project, new_project.path());
        assert_eq!(owner.snapshot.project, new_project.path());
        assert_eq!(owner.active_session, None);
        assert_eq!(owner.process_generation, 2);
        assert!(owner.process.is_some());
        assert!(owner.snapshot.connected);
        if let Some(mut process) = owner.process.take() {
            process.terminate()?;
        }
        Ok(())
    }

    #[test]
    fn background_catalog_refresh_preserves_search_until_user_clears_it()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let project = temp.path().join("project");
        fs::create_dir(&project)?;
        let alpha_path = temp.path().join("alpha.jsonl");
        let beta_path = temp.path().join("beta.jsonl");
        fs::write(&alpha_path, "{}")?;
        fs::write(&beta_path, "{}")?;
        let summary = |id: &str, path: PathBuf, search: &str| {
            SessionSummary::from_cached(
                id.into(),
                path,
                project.clone(),
                id.into(),
                String::new(),
                String::new(),
                None,
                SystemTime::now(),
                0,
                crate::sessions::UsageSummary::default(),
                false,
                false,
                search.into(),
            )
        };
        let sessions = vec![
            summary("alpha", alpha_path.canonicalize()?, "alpha"),
            summary("beta", beta_path.canonicalize()?, "beta"),
        ];
        let (mut owner, events, _discovery) = owner_without_process(project);
        owner.state = Some(StateStore::open_at(&temp.path().join("gui-state.sqlite3"))?);
        owner
            .state
            .as_mut()
            .expect("state")
            .replace_sessions(&sessions)?;

        owner.load_sessions("alpha".into());
        let searched = events.try_iter().collect::<Vec<_>>();
        assert!(searched.iter().any(|event| matches!(
            event,
            RuntimeEvent::Sessions { sessions, .. }
                if sessions.len() == 1 && sessions[0].id == "alpha"
        )));

        owner.refresh_sessions();
        let refresh_generation = owner.session_generation;
        assert_eq!(owner.session_query, "alpha");
        owner.apply_discovery(DiscoveryResult {
            generation: refresh_generation,
            result: Ok(SessionDiscovery {
                sessions,
                activities: HashMap::new(),
                exhaustive: true,
            }),
        });
        let refreshed = events.try_iter().collect::<Vec<_>>();
        assert!(refreshed.iter().any(|event| matches!(
            event,
            RuntimeEvent::Sessions { sessions, .. }
                if sessions.len() == 1 && sessions[0].id == "alpha"
        )));
        assert_eq!(owner.session_query, "alpha");

        owner.load_sessions(String::new());
        let cleared = events.try_iter().collect::<Vec<_>>();
        assert!(cleared.iter().any(|event| matches!(
            event,
            RuntimeEvent::Sessions { sessions, .. } if sessions.len() == 2
        )));
        assert!(owner.session_query.is_empty());
        Ok(())
    }

    #[test]
    fn in_flight_catalog_refreshes_coalesce_into_one_delayed_scan() {
        let (mut owner, _events, _discovery) = owner_without_process(std::env::temp_dir());
        owner.session_discovery_in_flight = true;
        owner.refresh_sessions();
        owner.refresh_sessions();
        assert!(owner.session_refresh_pending);

        owner.apply_discovery(DiscoveryResult {
            generation: 0,
            result: Ok(SessionDiscovery {
                sessions: Vec::new(),
                activities: HashMap::new(),
                exhaustive: true,
            }),
        });
        let due = owner.session_refresh_due.expect("deferred refresh");
        assert!(!owner.session_refresh_pending);
        owner.refresh_sessions();
        assert_eq!(owner.session_generation, 0);
        owner.poll_deferred_session_refresh(due - Duration::from_millis(1));
        assert_eq!(owner.session_generation, 0);

        owner.poll_deferred_session_refresh(due);
        assert_eq!(owner.session_generation, 1);
        assert!(owner.session_discovery_in_flight);
        owner.poll_deferred_session_refresh(due + Duration::from_secs(1));
        assert_eq!(owner.session_generation, 1);
    }

    #[test]
    fn cached_child_only_search_publishes_tree_closure_and_unfiltered_catalog()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let project = temp.path().join("project");
        fs::create_dir(&project)?;
        let root_path = temp.path().join("root.jsonl");
        let child_path = temp.path().join("child.jsonl");
        fs::write(&root_path, "{}")?;
        fs::write(&child_path, "{}")?;
        let root = SessionSummary::from_cached(
            "root".into(),
            root_path.canonicalize()?,
            project.clone(),
            "Main".into(),
            String::new(),
            String::new(),
            None,
            SystemTime::now(),
            0,
            crate::sessions::UsageSummary::default(),
            false,
            false,
            "ordinary".into(),
        );
        let child = SessionSummary::from_cached(
            "child".into(),
            child_path.canonicalize()?,
            project.clone(),
            "subagent-worker".into(),
            "Needle assignment".into(),
            String::new(),
            Some("root".into()),
            SystemTime::now(),
            0,
            crate::sessions::UsageSummary::default(),
            false,
            true,
            "needle assignment".into(),
        );
        let (mut owner, events, _) = owner_without_process(project);
        owner.state = Some(StateStore::open_at(&temp.path().join("gui-state.sqlite3"))?);
        owner
            .state
            .as_mut()
            .expect("state")
            .replace_sessions(&[root, child])?;

        owner.load_sessions("needle".into());

        assert!(events.try_iter().any(|event| matches!(
            event,
            RuntimeEvent::Sessions { sessions, all_sessions, .. }
                if sessions.iter().map(|session| session.id.as_str()).collect::<HashSet<_>>()
                    == HashSet::from(["root", "child"])
                    && all_sessions.len() == 2
        )));
        Ok(())
    }

    #[test]
    fn partial_catalog_refresh_keeps_omitted_running_children_in_polling_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let project = temp.path().join("project");
        fs::create_dir(&project)?;
        let root_path = temp.path().join("root.jsonl");
        let child_path = temp.path().join("child.jsonl");
        fs::write(&root_path, "{}")?;
        fs::write(&child_path, "{}")?;
        let summary = |id: &str, path: PathBuf, parent: Option<&str>, is_running: bool| {
            SessionSummary::from_cached(
                id.into(),
                path,
                project.clone(),
                id.into(),
                String::new(),
                String::new(),
                parent.map(str::to_owned),
                SystemTime::now(),
                0,
                crate::sessions::UsageSummary::default(),
                false,
                is_running,
                id.into(),
            )
        };
        let root = summary("root", root_path.canonicalize()?, None, false);
        let child = summary("child", child_path.canonicalize()?, Some("root"), true);
        let (mut owner, events, _discovery) = owner_without_process(project);
        owner.state = Some(StateStore::open_at(&temp.path().join("gui-state.sqlite3"))?);
        owner
            .state
            .as_mut()
            .expect("state")
            .replace_sessions(&[root.clone(), child])?;

        owner.apply_discovery(DiscoveryResult {
            generation: 0,
            result: Ok(SessionDiscovery {
                sessions: vec![root],
                activities: HashMap::new(),
                exhaustive: false,
            }),
        });

        assert!(events.try_iter().any(|event| matches!(
            event,
            RuntimeEvent::Sessions {
                has_running_descendants: true,
                ..
            }
        )));
        Ok(())
    }

    #[test]
    fn first_session_path_triggers_a_sidebar_refresh() {
        let project = std::env::temp_dir();
        let session = project.join("new-session.jsonl");
        let (mut owner, _events, _discovery) = owner_without_process(project);

        owner.apply_response(crate::protocol::RpcResponse {
            id: Some("state".into()),
            command: "get_state".into(),
            success: true,
            data: json!({
                "model": null,
                "thinkingLevel": "off",
                "isStreaming": true,
                "isCompacting": false,
                "sessionFile": session,
                "sessionId": "new-session",
                "sessionName": null,
                "autoCompactionEnabled": true,
                "messageCount": 1,
                "pendingMessageCount": 0
            }),
            error: None,
        });

        assert_eq!(owner.active_session, Some(session));
        assert_eq!(owner.session_generation, 1);
    }

    #[test]
    fn get_state_canonicalizes_a_symlinked_session_path() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempdir()?;
        let session = temp.path().join("session.jsonl");
        let link = temp.path().join("linked.jsonl");
        fs::write(&session, "{}")?;
        symlink(&session, &link)?;
        let (mut owner, _events, _discovery) = owner_without_process(temp.path().to_path_buf());

        owner.apply_response(crate::protocol::RpcResponse {
            id: Some("state".into()),
            command: "get_state".into(),
            success: true,
            data: json!({
                "model": null,
                "thinkingLevel": "off",
                "isStreaming": false,
                "isCompacting": false,
                "sessionFile": link,
                "sessionId": "session",
                "sessionName": null,
                "autoCompactionEnabled": true,
                "messageCount": 0,
                "pendingMessageCount": 0
            }),
            error: None,
        });

        assert_eq!(owner.active_session, Some(session.canonicalize()?));
        assert_eq!(
            owner.snapshot.selected_session,
            Some(session.canonicalize()?)
        );
        Ok(())
    }

    #[test]
    fn draft_can_restore_its_parked_blank_run_without_restarting_pi() {
        let project = std::env::temp_dir().join("draft-project");
        let (mut owner, _events, _discovery) = owner_without_process(project.clone());
        owner.snapshot = RuntimeSnapshot {
            history_preview: true,
            project: PathBuf::from("/other"),
            selected_session: Some(PathBuf::from("/other/session.jsonl")),
            ..RuntimeSnapshot::default()
        };
        owner.parked_snapshot = Some(RuntimeSnapshot {
            connected: true,
            status: "Idle".into(),
            project: project.clone(),
            ..RuntimeSnapshot::default()
        });
        let generation = owner.process_generation;

        owner.resume_draft(project.clone());

        assert_eq!(owner.project, project);
        assert_eq!(owner.snapshot.project, owner.project);
        assert!(!owner.snapshot.history_preview);
        assert!(owner.parked_snapshot.is_none());
        assert_eq!(owner.process_generation, generation);
    }

    #[test]
    fn starting_session_prefills_controls_from_the_last_ready_session() {
        let model = Model {
            id: "model-1".into(),
            name: "Model One".into(),
            provider: "provider-1".into(),
            reasoning: true,
        };
        let mut controls = SessionControls::default();
        let mut ready = RuntimeSnapshot {
            session: serde_json::from_value(json!({
                "model": {
                    "id": "model-1",
                    "name": "Model One",
                    "provider": "provider-1",
                    "reasoning": true
                },
                "thinkingLevel": "high",
                "isStreaming": false,
                "isCompacting": false,
                "sessionFile": "/old",
                "sessionId": "old",
                "autoCompactionEnabled": true,
                "messageCount": 0,
                "pendingMessageCount": 0
            }))
            .ok(),
            models: vec![model.clone()],
            thinking_levels: vec!["off".into(), "high".into()],
            ..RuntimeSnapshot::default()
        };
        prefill_session_controls(&mut ready, &mut controls);

        let mut starting = RuntimeSnapshot::default();
        prefill_session_controls(&mut starting, &mut controls);

        assert_eq!(starting.prefill_model, Some(model.clone()));
        assert_eq!(starting.prefill_thinking_level.as_deref(), Some("high"));
        assert_eq!(starting.models, vec![model]);
        assert_eq!(starting.thinking_levels, vec!["off", "high"]);
        assert!(starting.session.is_none());
    }

    #[test]
    fn process_replacement_clears_all_session_owned_snapshot_state() {
        let mut snapshot = RuntimeSnapshot {
            connected: true,
            status: "old".into(),
            session: serde_json::from_value(json!({
                "model": null,
                "thinkingLevel": "high",
                "isStreaming": true,
                "isCompacting": true,
                "sessionFile": "/old",
                "sessionId": "old",
                "autoCompactionEnabled": true,
                "messageCount": 9,
                "pendingMessageCount": 2
            }))
            .ok(),
            selected_session: Some(PathBuf::from("/old")),
            models: vec![Model {
                id: "old".into(),
                name: "Old".into(),
                provider: "test".into(),
                reasoning: false,
            }],
            thinking_levels: vec!["high".into()],
            stats: json!({"tokens": 10}),
            commands: vec![SlashCommand {
                name: "old".into(),
                description: None,
                source: crate::protocol::SlashCommandSource::Extension,
            }],
            stderr: "old stderr".into(),
            auto_retry: false,
            ..RuntimeSnapshot::default()
        };
        snapshot
            .conversation
            .reduce(&json!({"type": "agent_start"}));
        snapshot.conversation.reduce(&json!({
            "type": "queue_update",
            "steering": ["old"],
            "followUp": ["later"]
        }));

        reset_snapshot_for_process(
            &mut snapshot,
            PathBuf::from("/new-project"),
            Some(PathBuf::from("/new")),
            "Resuming session".into(),
        );

        assert!(!snapshot.connected);
        assert_eq!(snapshot.status, "Resuming session");
        assert_eq!(snapshot.project, PathBuf::from("/new-project"));
        assert_eq!(snapshot.selected_session, Some(PathBuf::from("/new")));
        assert!(!snapshot.auto_retry);
        assert_eq!(snapshot.session, None);
        assert!(snapshot.models.is_empty());
        assert!(snapshot.thinking_levels.is_empty());
        assert_eq!(snapshot.stats, Value::Null);
        assert!(snapshot.commands.is_empty());
        assert!(snapshot.stderr.is_empty());
        assert_eq!(snapshot.conversation, ConversationState::default());
    }

    #[test]
    fn failed_resume_publishes_no_state_from_the_previous_process() {
        let (event_tx, event_rx) = mpsc::channel();
        let (discovery_tx, _discovery_rx) = mpsc::channel();
        let (history_tx, _history_rx) = mpsc::channel();
        let mut owner = RuntimeOwner {
            project: std::env::temp_dir(),
            process_command: ProcessCommand {
                program: PathBuf::from("/definitely/missing/pi-gpui-test-command"),
                prefix_args: Vec::new(),
                direnv_program: None,
            },
            process: None,
            snapshot: RuntimeSnapshot {
                connected: true,
                status: "old".into(),
                selected_session: Some(PathBuf::from("/old")),
                models: vec![Model {
                    id: "old".into(),
                    name: "Old".into(),
                    provider: "test".into(),
                    reasoning: false,
                }],
                thinking_levels: vec!["high".into()],
                stats: json!({"old": true}),
                commands: vec![SlashCommand {
                    name: "old".into(),
                    description: None,
                    source: crate::protocol::SlashCommandSource::Extension,
                }],
                stderr: "old stderr".into(),
                auto_retry: true,
                ..RuntimeSnapshot::default()
            },
            owns_session_catalog: false,
            session_generation: 0,
            session_discovery_in_flight: false,
            session_refresh_pending: false,
            session_refresh_due: None,
            process_generation: 4,
            pending_prompt_id: None,
            pending_prompt_target: None,
            pending_prompt_item: None,
            pending_outbox_id: None,
            event_tx,
            discovery_tx,
            history_tx,
            history_generation: 0,
            active_session: Some(PathBuf::from("/old")),
            parked_snapshot: None,
            deferred_prompt: None,
            startup_state_loaded: false,
            startup_history_loaded: false,
            state: None,
            session_query: String::new(),
        };
        owner.snapshot.conversation.reduce(&json!({
            "type": "queue_update",
            "steering": ["old"],
            "followUp": []
        }));

        owner.start_process(Some(PathBuf::from("/new")));

        let snapshots = event_rx
            .try_iter()
            .filter_map(|event| match event {
                RuntimeEvent::Snapshot { snapshot, .. } => Some(snapshot),
                _ => None,
            })
            .collect::<Vec<_>>();
        let latest = snapshots.last().expect("failed replacement should publish");
        assert_eq!(latest.selected_session, Some(PathBuf::from("/new")));
        assert!(latest.models.is_empty());
        assert!(latest.thinking_levels.is_empty());
        assert_eq!(latest.stats, Value::Null);
        assert!(latest.commands.is_empty());
        assert!(latest.stderr.is_empty());
        assert!(latest.conversation.queue.steering.is_empty());
        assert!(
            latest
                .conversation
                .items
                .iter()
                .any(|item| item.text == "Try again from the composer.")
        );
        assert!(
            latest
                .conversation
                .diagnostics
                .iter()
                .any(|item| item.contains("definitely/missing"))
        );
    }

    #[test]
    fn history_preview_keeps_pi_until_a_prompt_resumes_the_session() -> Result<(), String> {
        let temp = tempdir().map_err(|error| error.to_string())?;
        let script = temp.path().join("fake-pi.sh");
        fs::write(&script, include_str!("../tests/fixtures/fake-pi.sh"))
            .map_err(|error| error.to_string())?;
        let mut permissions = fs::metadata(&script)
            .map_err(|error| error.to_string())?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).map_err(|error| error.to_string())?;
        let process_command = ProcessCommand {
            program: script,
            prefix_args: vec!["quiet".into()],
            direnv_program: None,
        };
        let process = RpcProcess::spawn(&process_command, temp.path(), None)?;
        let (event_tx, event_rx) = mpsc::channel();
        let (discovery_tx, _discovery_rx) = mpsc::channel();
        let (history_tx, _history_rx) = mpsc::channel();
        let old_path = PathBuf::from("/old");
        let new_path = PathBuf::from("/new");
        let old_project = temp.path().to_path_buf();
        let new_project = temp.path().join("other-project");
        fs::create_dir(&new_project).map_err(|error| error.to_string())?;
        let mut owner = RuntimeOwner {
            project: old_project.clone(),
            process_command,
            process: Some(process),
            snapshot: RuntimeSnapshot {
                connected: true,
                status: "Ready".into(),
                project: old_project.clone(),
                selected_session: Some(old_path.clone()),
                ..RuntimeSnapshot::default()
            },
            owns_session_catalog: false,
            session_generation: 0,
            session_discovery_in_flight: false,
            session_refresh_pending: false,
            session_refresh_due: None,
            process_generation: 3,
            pending_prompt_id: None,
            pending_prompt_target: None,
            pending_prompt_item: None,
            pending_outbox_id: None,
            event_tx,
            discovery_tx,
            history_tx,
            history_generation: 1,
            active_session: Some(old_path.clone()),
            parked_snapshot: None,
            deferred_prompt: None,
            startup_state_loaded: false,
            startup_history_loaded: false,
            state: Some(StateStore::open_at(&temp.path().join("gui-state.sqlite3"))?),
            session_query: String::new(),
        };

        owner.preview_history(old_path.clone(), old_project);
        owner.apply_history(HistoryResult {
            generation: 1,
            path: new_path.clone(),
            project: new_project.clone(),
            result: Ok(vec![json!({"role":"user","content":"previewed"})]),
        });
        assert!(!owner.snapshot.history_preview);
        owner.apply_history(HistoryResult {
            generation: 2,
            path: new_path.clone(),
            project: new_project.clone(),
            result: Ok(vec![json!({"role":"user","content":"previewed"})]),
        });

        assert!(owner.process.is_some());
        assert!(owner.snapshot.history_preview);
        assert_eq!(owner.snapshot.project, new_project);
        assert_eq!(owner.project, owner.snapshot.project);
        assert_eq!(owner.snapshot.selected_session, Some(new_path.clone()));
        assert_eq!(
            owner
                .parked_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.selected_session.clone()),
            Some(old_path)
        );

        let _ = event_rx.try_iter().count();
        owner.send_prompt(
            format!("session:{}", new_path.display()),
            PromptMode::Normal,
            "continue".into(),
            Vec::new(),
            false,
        );
        assert!(owner.snapshot.history_preview);
        assert_eq!(owner.snapshot.conversation.items[0].text, "previewed");
        assert_eq!(owner.active_session, Some(new_path.clone()));
        assert!(owner.deferred_prompt.is_some());
        let resume_events = event_rx.try_iter().collect::<Vec<_>>();
        assert!(
            resume_events
                .iter()
                .all(|event| !matches!(event, RuntimeEvent::PromptResult { accepted: true, .. }))
        );
        assert!(resume_events.iter().any(|event| matches!(
            event,
            RuntimeEvent::SessionReset {
                preserve_submission: true,
                ..
            }
        )));
        assert!(
            resume_events
                .iter()
                .filter_map(|event| match event {
                    RuntimeEvent::Snapshot { snapshot, .. } => Some(snapshot),
                    _ => None,
                })
                .all(|snapshot| {
                    snapshot.history_preview
                        && snapshot
                            .conversation
                            .items
                            .first()
                            .map(|item| item.text.as_str())
                            == Some("previewed")
                })
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            while let Some(item) = owner.process.as_mut().and_then(RpcProcess::try_next) {
                owner.apply_process_item(item);
            }
            if owner.deferred_prompt.is_none() && owner.pending_prompt_id.is_none() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(owner.deferred_prompt.is_none());
        assert!(owner.pending_prompt_id.is_none());
        assert!(!owner.snapshot.history_preview);
        assert!(owner.snapshot.conversation.items.iter().any(|item| {
            item.kind == crate::conversation::TranscriptKind::User && item.text == "continue"
        }));
        assert!(event_rx.try_iter().any(|event| matches!(
            event,
            RuntimeEvent::PromptResult {
                target,
                accepted: true,
                ..
            } if target == format!("session:{}", new_path.display())
        )));
        Ok(())
    }

    #[test]
    fn active_session_events_stay_parked_while_other_history_is_visible() -> Result<(), String> {
        let temp = tempdir().map_err(|error| error.to_string())?;
        let active_path = temp.path().join("active.jsonl");
        let history_path = temp.path().join("history.jsonl");
        let history_project = temp.path().join("history-project");
        fs::create_dir(&history_project).map_err(|error| error.to_string())?;
        fs::write(
            &history_path,
            format!(
                "{{\"type\":\"session\",\"id\":\"history\",\"cwd\":{},\"timestamp\":\"2026-08-15T00:00:00Z\"}}\n{{\"type\":\"message\",\"id\":\"one\",\"parentId\":null,\"message\":{{\"role\":\"user\",\"content\":\"history message\"}}}}\n",
                serde_json::to_string(&history_project).map_err(|error| error.to_string())?
            ),
        )
        .map_err(|error| error.to_string())?;

        let (event_tx, event_rx) = mpsc::channel();
        let (discovery_tx, _discovery_rx) = mpsc::channel();
        let (history_tx, history_rx) = mpsc::channel();
        let mut owner = RuntimeOwner {
            project: temp.path().to_path_buf(),
            process_command: ProcessCommand::default(),
            process: None,
            snapshot: RuntimeSnapshot {
                connected: true,
                status: "Working".into(),
                project: temp.path().to_path_buf(),
                selected_session: Some(active_path.clone()),
                ..RuntimeSnapshot::default()
            },
            owns_session_catalog: false,
            session_generation: 0,
            session_discovery_in_flight: false,
            session_refresh_pending: false,
            session_refresh_due: None,
            process_generation: 7,
            pending_prompt_id: Some("pending-prompt".into()),
            pending_prompt_target: Some(format!("session:{}", active_path.display())),
            pending_prompt_item: None,
            pending_outbox_id: None,
            event_tx,
            discovery_tx,
            history_tx,
            history_generation: 0,
            active_session: Some(active_path.clone()),
            parked_snapshot: None,
            deferred_prompt: None,
            startup_state_loaded: true,
            startup_history_loaded: true,
            state: None,
            session_query: String::new(),
        };
        owner
            .snapshot
            .conversation
            .reduce(&json!({"type":"agent_start"}));

        owner.preview_history(history_path.clone(), history_project.clone());
        let history = history_rx
            .recv_timeout(Duration::from_secs(1))
            .map_err(|error| format!("history switch was rejected: {error}"))?;
        owner.apply_history(history);

        assert!(owner.snapshot.history_preview);
        assert_eq!(owner.snapshot.selected_session, Some(history_path));
        assert_eq!(owner.snapshot.conversation.items[0].text, "history message");
        assert!(
            owner
                .parked_snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.conversation.running)
        );
        let visible = event_rx
            .try_iter()
            .filter_map(|event| match event {
                RuntimeEvent::Snapshot { snapshot, .. } => Some(snapshot),
                _ => None,
            })
            .last()
            .expect("history preview should publish");
        assert_eq!(visible.live_session, Some(active_path.clone()));
        assert_eq!(visible.live_status, "Working");
        assert_eq!(visible.conversation.items[0].text, "history message");

        assert!(!owner.apply_process_item(ProcessItem::Event(json!({
            "type": "message_start",
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "active output"}]
            }
        }))));

        assert_eq!(owner.snapshot.conversation.items[0].text, "history message");
        assert!(owner.parked_snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .conversation
                .items
                .iter()
                .any(|item| item.text == "active output")
        }));
        assert!(
            event_rx
                .try_iter()
                .all(|event| !matches!(event, RuntimeEvent::Snapshot { .. }))
        );

        let changed = owner.apply_process_item(ProcessItem::Event(json!({
            "type": "compaction_start",
            "reason": "test"
        })));
        assert!(changed);
        owner.publish();
        let visible = event_rx
            .try_iter()
            .filter_map(|event| match event {
                RuntimeEvent::Snapshot { snapshot, .. } => Some(snapshot),
                _ => None,
            })
            .last()
            .expect("live badge change should publish");
        assert_eq!(visible.live_session, Some(active_path.clone()));
        assert_eq!(visible.live_status, "Compacting");
        assert_eq!(visible.conversation.items[0].text, "history message");

        owner.preview_history(active_path.clone(), temp.path().to_path_buf());
        assert!(!owner.snapshot.history_preview);
        assert_eq!(owner.snapshot.selected_session, Some(active_path));
        assert!(
            owner
                .snapshot
                .conversation
                .items
                .iter()
                .any(|item| item.text == "active output")
        );
        Ok(())
    }
}
