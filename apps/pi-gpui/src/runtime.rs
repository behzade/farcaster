//! UI-neutral application runtime and active-session ownership.

mod catalog;
mod documents;
mod permission_level;
mod prompts;
mod session_controls;
mod session_identity;

pub(crate) use permission_level::PermissionLevel;

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
    conversation::{ConversationState, TranscriptItem, TranscriptKind},
    protocol::{
        ExtensionUiRequest, ExtensionUiResponse, Model, PromptImage, PromptMode, SessionState,
        SlashCommand, command,
    },
    rpc_process::{ProcessCommand, ProcessItem, RpcProcess},
    session_transfer::{self, TransferMember},
    session_watcher::{SessionWatchEvent, SessionWatcher},
    sessions::{
        LoadedHistory, RUNNING_ACTIVITY_TIMEOUT, SessionDiscovery, SessionSummary,
        configured_session_root, discover, load_history, project_display_history,
        session_family_for_path,
    },
    state::StateStore,
};
use catalog::ExternalActivityTracker;
use session_controls::PendingSessionControls;
use session_identity::SessionControlDefaults;

const COALESCED_SESSION_REFRESH_DELAY: Duration = Duration::from_millis(100);
const STREAM_PUBLISH_INTERVAL: Duration = Duration::from_millis(16);
const MAX_FAILURE_DETAILS_CHARS: usize = 12_000;
const MAX_FAILURE_SUMMARY_CHARS: usize = 240;

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
    StopSessionFamily {
        path: PathBuf,
    },
    DeleteSessionFamily {
        path: PathBuf,
    },
    Reload,
    Compact {
        custom_instructions: Option<String>,
    },
    ExportHtml {
        output_path: Option<String>,
    },
    SetSessionName(String),
    RenameSession {
        path: PathBuf,
        project: PathBuf,
        name: String,
    },
    MoveSession {
        path: PathBuf,
        target_project: PathBuf,
    },
    NewSession {
        id: String,
        project: PathBuf,
    },
    ResumeDraft {
        id: String,
        project: PathBuf,
    },
    SelectSession {
        path: PathBuf,
        project: PathBuf,
    },
    RestartSession {
        path: PathBuf,
        project: PathBuf,
    },
    RefreshSessionDocument {
        path: PathBuf,
        project: PathBuf,
    },
    SetModel {
        provider: String,
        model_id: String,
    },
    Login(Option<String>),
    SetThinking(String),
    SetPermissionLevel(PermissionLevel),
    ExtensionResponse(ExtensionUiResponse),
    DeliverQueued(crate::state::QueuedPrompt),
    SetSessionCategory {
        path: PathBuf,
        in_review: bool,
        archived: bool,
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
    },
    SessionsFailed {
        generation: u64,
        message: String,
    },
    SessionMoved {
        target_root: PathBuf,
        target_project: PathBuf,
        paths: Arc<HashMap<PathBuf, PathBuf>>,
    },
    SessionDeleted {
        generation: u64,
        paths: Arc<HashSet<PathBuf>>,
    },
    RefreshCatalog,
    WorkGraphChanged {
        project: PathBuf,
    },
    ExtensionUi {
        generation: u64,
        request: crate::protocol::ExtensionUiRequest,
        system_notification_target: Option<(PathBuf, PathBuf)>,
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
    SessionFilesModified {
        paths: Vec<PathBuf>,
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
    pub conversation: Arc<ConversationState>,
    pub models: Vec<Model>,
    pub thinking_levels: Vec<String>,
    pub stats: Value,
    pub commands: Vec<SlashCommand>,
    pub stderr: String,
    pub auto_retry: bool,
    pub permission_level: PermissionLevel,
    pub history_preview: bool,
    pub pending_question: Option<ExtensionUiRequest>,
    pub transcript_changed_from: Option<usize>,
}

pub(crate) struct RuntimeHandle {
    commands: mpsc::Sender<RuntimeCommand>,
    events: mpsc::Receiver<RuntimeEvent>,
    wake: async_channel::Receiver<()>,
    thread: thread::Thread,
    join: Option<thread::JoinHandle<()>>,
}

#[derive(Clone)]
struct UiEventSender {
    events: mpsc::Sender<RuntimeEvent>,
    wake: async_channel::Sender<()>,
}

impl UiEventSender {
    fn send(&self, event: RuntimeEvent) -> Result<(), ()> {
        self.events.send(event).map_err(|_| ())?;
        let _ = self.wake.try_send(());
        Ok(())
    }
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
        let (events_tx, events) = mpsc::channel();
        let (wake_tx, wake) = async_channel::bounded(1);
        let event_tx = UiEventSender {
            events: events_tx,
            wake: wake_tx,
        };
        let handle = thread::Builder::new()
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
            .expect("start Pi supervisor");
        Self {
            commands,
            events,
            wake,
            thread: handle.thread().clone(),
            join: Some(handle),
        }
    }

    pub(crate) fn send(&self, command: RuntimeCommand) -> Result<(), String> {
        self.commands
            .send(command)
            .map_err(|_| "Pi runtime has stopped".to_owned())?;
        self.thread.unpark();
        Ok(())
    }

    pub(crate) fn try_recv(&self) -> Result<RuntimeEvent, mpsc::TryRecvError> {
        self.events.try_recv()
    }

    pub(crate) fn wake_receiver(&self) -> async_channel::Receiver<()> {
        self.wake.clone()
    }
}

impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        let _ = self.commands.send(RuntimeCommand::Shutdown);
        self.thread.unpark();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

use documents::reconcile_live_session_documents;
use prompts::DeferredPrompt;

#[derive(Clone)]
struct SessionEventSender {
    sender: mpsc::Sender<RuntimeEvent>,
    supervisor: thread::Thread,
}

impl SessionEventSender {
    fn send(&self, event: RuntimeEvent) -> Result<(), ()> {
        self.sender.send(event).map_err(|_| ())?;
        self.supervisor.unpark();
        Ok(())
    }
}

struct SessionRuntimeHandle {
    commands: mpsc::Sender<RuntimeCommand>,
    events: mpsc::Receiver<RuntimeEvent>,
    thread: thread::Thread,
    join: thread::JoinHandle<()>,
}

impl SessionRuntimeHandle {
    fn spawn(
        project: PathBuf,
        process_command: ProcessCommand,
        load_catalog: bool,
        supervisor: thread::Thread,
    ) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (event_sender, events) = mpsc::channel();
        let event_tx = SessionEventSender {
            sender: event_sender,
            supervisor,
        };
        let handle = thread::Builder::new()
            .name("pi-gpui-session".into())
            .spawn(move || run(project, process_command, command_rx, event_tx, load_catalog))
            .expect("start Pi session runtime");
        Self {
            commands,
            events,
            thread: handle.thread().clone(),
            join: handle,
        }
    }

    fn send(&self, command: RuntimeCommand) {
        if self.commands.send(command).is_ok() {
            self.thread.unpark();
        }
    }

    fn join(self) {
        let _ = self.join.join();
    }
}

fn publish_session_status_if_changed(
    sender: &UiEventSender,
    published: &mut HashMap<String, (Option<PathBuf>, String)>,
    target: &str,
    session: Option<PathBuf>,
    status: &str,
) {
    let next = (session.clone(), status.to_owned());
    if published.get(target) == Some(&next) {
        return;
    }
    published.insert(target.to_owned(), next);
    let _ = sender.send(RuntimeEvent::SessionStatus {
        target: target.to_owned(),
        session,
        status: status.to_owned(),
    });
}

fn rpc_owned_session_paths(latest: &HashMap<String, Arc<RuntimeSnapshot>>) -> HashSet<PathBuf> {
    latest
        .values()
        .filter(|snapshot| snapshot.connected)
        .filter_map(|snapshot| {
            let live = snapshot.live_session.as_ref()?;
            (!snapshot.history_preview || snapshot.selected_session.as_ref() != Some(live))
                .then(|| live.clone())
        })
        .collect()
}

fn run_supervisor(
    project: PathBuf,
    draft_id: String,
    initial_session: Option<PathBuf>,
    process_command: ProcessCommand,
    command_rx: mpsc::Receiver<RuntimeCommand>,
    event_tx: UiEventSender,
) {
    let supervisor_thread = thread::current();
    let initial_key = format!("draft:{draft_id}");
    let catalog_key = "catalog".to_owned();
    let initial_project = project.clone();
    let mut actors = HashMap::from([
        (
            catalog_key.clone(),
            SessionRuntimeHandle::spawn(
                project.clone(),
                process_command.clone(),
                true,
                supervisor_thread.clone(),
            ),
        ),
        (
            initial_key.clone(),
            SessionRuntimeHandle::spawn(
                project,
                process_command.clone(),
                false,
                supervisor_thread.clone(),
            ),
        ),
    ]);
    if let Some(actor) = actors.get(&initial_key) {
        actor.send(initial_draft_command(
            draft_id,
            initial_project.clone(),
            initial_session.clone(),
        ));
    }
    let mut selected = initial_key.clone();
    let mut generation = 0_u64;
    let mut latest = HashMap::<String, Arc<RuntimeSnapshot>>::new();
    let mut catalog_sessions = Vec::<SessionSummary>::new();
    let mut catalog_generation = 0_u64;
    let mut catalog_exhaustive = false;
    let mut activity_tracker = ExternalActivityTracker::default();
    if let Some(path) = initial_session.clone() {
        latest.insert(
            initial_key.clone(),
            Arc::new(RuntimeSnapshot {
                project: initial_project,
                selected_session: Some(path),
                history_preview: true,
                ..RuntimeSnapshot::default()
            }),
        );
    }
    let mut actor_paths = initial_session
        .map(|path| HashMap::from([(path, initial_key.clone())]))
        .unwrap_or_default();
    let mut interacted = HashSet::from([initial_key.clone()]);
    let mut document_revisions = HashMap::new();
    let mut pending_extensions = HashMap::<String, Vec<crate::protocol::ExtensionUiRequest>>::new();
    let mut active_dialogs = HashMap::<String, Vec<crate::protocol::ExtensionUiRequest>>::new();
    let mut needs_input = HashSet::<String>::new();
    let mut clock = 0_u64;
    let mut last_touch = HashMap::from([(initial_key.clone(), clock)]);
    let mut session_controls = SessionControlDefaults::default();
    let mut published_statuses = HashMap::<String, (Option<PathBuf>, String)>::new();
    if let Ok(state) = StateStore::open()
        && let Ok(prompts) = state.queued_prompts()
    {
        for prompt in prompts {
            let key = prompt.target.clone();
            let actor = actors.entry(key).or_insert_with(|| {
                SessionRuntimeHandle::spawn(
                    prompt.project.clone(),
                    process_command.clone(),
                    false,
                    supervisor_thread.clone(),
                )
            });
            actor.send(RuntimeCommand::DeliverQueued(prompt));
        }
    }
    let mut running = true;
    while running {
        let owned_sessions = rpc_owned_session_paths(&latest);
        activity_tracker.remove_owned(&owned_sessions);
        if activity_tracker.take_expired(Instant::now())
            && let Some(catalog) = actors.get(&catalog_key)
        {
            catalog.send(RuntimeCommand::RefreshSessions);
        }
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
                        let session_path = snapshot
                            .live_session
                            .clone()
                            .or_else(|| snapshot.selected_session.clone());
                        let adopts_identity = key == selected
                            && !session_path.is_some_and(|path| {
                                crate::sessions::is_subagent_path(&catalog_sessions, &path)
                            });
                        session_controls.apply(Arc::make_mut(&mut snapshot), adopts_identity);
                        if snapshot.conversation.settled {
                            needs_input.remove(&key);
                            active_dialogs.remove(&key);
                        }
                        let status = if needs_input.contains(&key) {
                            "Needs input"
                        } else {
                            semantic_status(&snapshot)
                        };
                        publish_session_status_if_changed(
                            &event_tx,
                            &mut published_statuses,
                            &key,
                            snapshot
                                .live_session
                                .clone()
                                .or_else(|| snapshot.selected_session.clone()),
                            status,
                        );
                        if let Some(path) = snapshot
                            .live_session
                            .clone()
                            .or_else(|| snapshot.selected_session.clone())
                        {
                            actor_paths.insert(path, key.clone());
                        }
                        latest.insert(key.clone(), snapshot.clone());
                        if key == selected {
                            let _ = event_tx.send(RuntimeEvent::Snapshot {
                                generation,
                                snapshot,
                            });
                        }
                    }
                    RuntimeEvent::ExtensionUi { request, .. } => {
                        if request.gpui_system_notification().is_some() {
                            let system_notification_target = latest
                                .get(&key)
                                .and_then(|snapshot| notification_target(snapshot));
                            let _ = event_tx.send(RuntimeEvent::ExtensionUi {
                                generation,
                                request,
                                system_notification_target,
                            });
                            continue;
                        }
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
                            publish_session_status_if_changed(
                                &event_tx,
                                &mut published_statuses,
                                &key,
                                session,
                                "Needs input",
                            );
                        }
                        if key == selected {
                            let _ = event_tx.send(RuntimeEvent::ExtensionUi {
                                generation,
                                request,
                                system_notification_target: None,
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
                        }
                    }
                    RuntimeEvent::SessionFilesModified { paths } if key == catalog_key => {
                        let refresh = activity_tracker.observe_files(
                            &catalog_sessions,
                            &rpc_owned_session_paths(&latest),
                            &paths,
                            Instant::now(),
                        );
                        if refresh && let Some(catalog) = actors.get(&catalog_key) {
                            catalog.send(RuntimeCommand::RefreshSessions);
                        }
                    }
                    RuntimeEvent::WorkGraphChanged { project, .. } => {
                        let _ = event_tx.send(RuntimeEvent::WorkGraphChanged { project });
                    }
                    event @ (RuntimeEvent::Sessions { .. }
                    | RuntimeEvent::SessionsFailed { .. }) => {
                        if key == catalog_key
                            && matches!(&event, RuntimeEvent::SessionsFailed { .. })
                        {
                            catalog_exhaustive = false;
                        }
                        if key == catalog_key
                            && let RuntimeEvent::Sessions {
                                generation: next_generation,
                                all_sessions,
                                activities,
                                ..
                            } = &event
                        {
                            catalog_generation = *next_generation;
                            if let Some((_, exhaustive)) = activities {
                                catalog_exhaustive = *exhaustive;
                                activity_tracker.sync_catalog(
                                    all_sessions,
                                    *exhaustive,
                                    &rpc_owned_session_paths(&latest),
                                    Instant::now(),
                                    SystemTime::now(),
                                );
                            }
                            catalog_sessions.clone_from(all_sessions);
                            reconcile_live_session_documents(
                                all_sessions,
                                &interacted,
                                &selected,
                                &mut actors,
                                &mut latest,
                                &mut last_touch,
                                &mut document_revisions,
                                &mut actor_paths,
                                &process_command,
                                &supervisor_thread,
                            );
                        }
                        match route_session_discovery(&key, &catalog_key, event) {
                            SupervisorSessionAction::Publish(event) => {
                                let _ = event_tx.send(event);
                            }
                            SupervisorSessionAction::RefreshCatalog => {
                                if let Some(catalog) = actors.get(&catalog_key) {
                                    catalog.send(RuntimeCommand::RefreshSessions);
                                }
                            }
                        }
                    }
                    RuntimeEvent::Stopped
                    | RuntimeEvent::SessionMoved { .. }
                    | RuntimeEvent::SessionDeleted { .. }
                    | RuntimeEvent::SessionStatus { .. }
                    | RuntimeEvent::SessionFilesModified { .. }
                    | RuntimeEvent::SessionReset { .. }
                    | RuntimeEvent::HistoryReset { .. } => {}
                }
            }
        }
        match command_rx.try_recv() {
            Ok(RuntimeCommand::Shutdown) => running = false,
            Ok(command) => {
                if let RuntimeCommand::StopSessionFamily { path } = &command {
                    if let Some(family) = session_family_for_path(&catalog_sessions, path) {
                        let family_paths = family
                            .iter()
                            .map(|session| session.path.clone())
                            .collect::<HashSet<_>>();
                        let family_actor_keys = actor_paths
                            .iter()
                            .filter(|(path, key)| {
                                family_paths.contains(*path) && *key != &catalog_key
                            })
                            .map(|(_, key)| key.clone())
                            .collect::<HashSet<_>>();
                        for key in &family_actor_keys {
                            if let Some(actor) = actors.remove(key) {
                                actor.send(RuntimeCommand::Shutdown);
                                actor.join();
                            }
                            latest.remove(key);
                            last_touch.remove(key);
                            pending_extensions.remove(key);
                            active_dialogs.remove(key);
                            needs_input.remove(key);
                            interacted.remove(key);
                            published_statuses.remove(key);
                        }
                        document_revisions.retain(|path, _| !family_paths.contains(path));
                        actor_paths.retain(|path, _| !family_paths.contains(path));
                        if family_actor_keys.contains(&selected) {
                            selected = catalog_key.clone();
                        }
                        if let Some(catalog) = actors.get(&catalog_key) {
                            catalog.send(RuntimeCommand::RefreshSessions);
                        }
                    }
                    continue;
                }
                if let RuntimeCommand::DeleteSessionFamily { path } = &command {
                    let result = (|| {
                        if !catalog_exhaustive {
                            return Err(
                                "Wait for a complete session scan before deleting this session"
                                    .to_owned(),
                            );
                        }
                        let requested = catalog_sessions
                            .iter()
                            .find(|session| session.path == *path)
                            .ok_or_else(|| {
                                "The session is no longer available to delete".to_owned()
                            })?;
                        if requested.parent_session.is_some() {
                            return Err("Only a root session can be deleted".to_owned());
                        }
                        let family = session_family_for_path(&catalog_sessions, path)
                            .expect("requested session belongs to the catalog");
                        let root = family[0];
                        if root.path != *path {
                            return Err("Only a root session can be deleted".to_owned());
                        }
                        if family.iter().any(|session| session.is_running) {
                            return Err("Wait for the session family to finish before deleting it"
                                .to_owned());
                        }
                        let family_paths = family
                            .iter()
                            .map(|session| session.path.clone())
                            .collect::<HashSet<_>>();
                        let family_actor_keys = actor_paths
                            .iter()
                            .filter(|(path, key)| {
                                family_paths.contains(*path) && *key != &catalog_key
                            })
                            .map(|(_, key)| key.clone())
                            .collect::<HashSet<_>>();
                        if family_actor_keys.iter().any(|key| {
                            latest.get(key).is_some_and(|snapshot| {
                                snapshot.conversation.running
                                    || snapshot.conversation.compacting
                                    || needs_input.contains(key)
                            })
                        }) {
                            return Err(
                                "Wait for the session family to become idle before deleting it"
                                    .to_owned(),
                            );
                        }
                        let mut state = StateStore::open()?;
                        let paths = family_paths.iter().cloned().collect::<Vec<_>>();
                        if state.has_queued_prompts_for(&paths)? {
                            return Err(
                                "Send or remove queued prompts before deleting this session"
                                    .to_owned(),
                            );
                        }
                        for key in &family_actor_keys {
                            if let Some(actor) = actors.remove(key) {
                                actor.send(RuntimeCommand::Shutdown);
                                actor.join();
                            }
                            latest.remove(key);
                            last_touch.remove(key);
                            pending_extensions.remove(key);
                            active_dialogs.remove(key);
                            needs_input.remove(key);
                            interacted.remove(key);
                            published_statuses.remove(key);
                        }
                        document_revisions.retain(|path, _| !family_paths.contains(path));
                        actor_paths.retain(|path, _| !family_paths.contains(path));
                        if family_actor_keys.contains(&selected) {
                            selected = catalog_key.clone();
                            generation = generation.saturating_add(1);
                        }
                        let leftovers = crate::session_deletion::delete_family(&paths)?;
                        let state_warning = state.delete_session_state(&paths).err();
                        Ok((family_paths, leftovers, state_warning))
                    })();
                    match result {
                        Ok((paths, leftovers, state_warning)) => {
                            let _ = event_tx.send(RuntimeEvent::SessionDeleted {
                                generation,
                                paths: Arc::new(paths),
                            });
                            let mut warnings = Vec::new();
                            if !leftovers.is_empty() {
                                warnings.push(format!(
                                    "some session files remain quarantined and must be removed manually: {}",
                                    leftovers
                                        .iter()
                                        .map(|(path, error)| {
                                            format!("{} ({error})", path.display())
                                        })
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ));
                            }
                            if let Some(message) = state_warning {
                                warnings.push(format!(
                                    "its saved UI state could not be removed: {message}"
                                ));
                            }
                            if !warnings.is_empty() {
                                let _ = event_tx.send(RuntimeEvent::SessionsFailed {
                                    generation: catalog_generation,
                                    message: format!(
                                        "Session deleted, but {}",
                                        warnings.join("; ")
                                    ),
                                });
                            }
                            if let Some(catalog) = actors.get(&catalog_key) {
                                catalog.send(RuntimeCommand::RefreshSessions);
                            }
                        }
                        Err(message) => {
                            let _ = event_tx.send(RuntimeEvent::SessionsFailed {
                                generation: catalog_generation,
                                message,
                            });
                        }
                    }
                    continue;
                }
                if let RuntimeCommand::MoveSession {
                    path,
                    target_project,
                } = &command
                {
                    let result = (|| {
                        let family =
                            session_family_for_path(&catalog_sessions, path).ok_or_else(|| {
                                "The session is no longer available to move".to_owned()
                            })?;
                        let root = family[0];
                        if root.path != *path {
                            return Err("Only a root session can be moved".to_owned());
                        }
                        if family.iter().any(|session| session.is_running) {
                            return Err(
                                "Wait for the session family to finish before moving it".to_owned()
                            );
                        }
                        let family_paths = family
                            .iter()
                            .map(|session| session.path.clone())
                            .collect::<HashSet<_>>();
                        let family_actor_keys = actor_paths
                            .iter()
                            .filter(|(path, key)| {
                                family_paths.contains(*path) && *key != &catalog_key
                            })
                            .map(|(_, key)| key.clone())
                            .collect::<HashSet<_>>();
                        if family_actor_keys.iter().any(|key| {
                            latest.get(key).is_some_and(|snapshot| {
                                snapshot.conversation.running
                                    || snapshot.conversation.compacting
                                    || needs_input.contains(key)
                            })
                        }) {
                            return Err(
                                "Wait for the session family to become idle before moving it"
                                    .to_owned(),
                            );
                        }
                        let mut state = StateStore::open()?;
                        let paths = family_paths.iter().cloned().collect::<Vec<_>>();
                        if state.has_queued_prompts_for(&paths)? {
                            return Err("Send or remove queued prompts before moving this session"
                                .to_owned());
                        }
                        for key in &family_actor_keys {
                            if let Some(actor) = actors.remove(key) {
                                actor.send(RuntimeCommand::Shutdown);
                                actor.join();
                            }
                            latest.remove(key);
                            last_touch.remove(key);
                            pending_extensions.remove(key);
                            active_dialogs.remove(key);
                            needs_input.remove(key);
                            interacted.remove(key);
                            published_statuses.remove(key);
                        }
                        document_revisions.retain(|path, _| !family_paths.contains(path));
                        actor_paths.retain(|path, _| !family_paths.contains(path));
                        let source_was_selected = family_actor_keys.contains(&selected);
                        if source_was_selected {
                            selected = catalog_key.clone();
                            generation = generation.saturating_add(1);
                        }
                        let members = family
                            .iter()
                            .map(|session| TransferMember {
                                path: session.path.clone(),
                                id: session.id.clone(),
                                parent_id: session.parent_session.clone(),
                            })
                            .collect::<Vec<_>>();
                        let session_root = configured_session_root()?;
                        let destination = session_transfer::destination_directory(
                            &session_root,
                            target_project,
                            &root.path,
                        );
                        let moved = session_transfer::move_family(
                            &members,
                            &root.id,
                            target_project,
                            &destination,
                        )?;
                        let path_updates = moved
                            .paths
                            .iter()
                            .map(|(source, target)| (source.clone(), target.clone()))
                            .collect::<Vec<_>>();
                        let state_warning = state
                            .relocate_session_paths(&path_updates, target_project)
                            .err();
                        Ok((moved, state_warning))
                    })();
                    match result {
                        Ok((moved, state_warning)) => {
                            let _ = event_tx.send(RuntimeEvent::SessionMoved {
                                target_root: moved.root,
                                target_project: target_project.clone(),
                                paths: Arc::new(moved.paths),
                            });
                            if let Some(message) = state_warning {
                                let _ = event_tx.send(RuntimeEvent::SessionsFailed {
                                    generation: catalog_generation,
                                    message: format!(
                                        "Session moved, but its saved UI state could not be migrated: {message}"
                                    ),
                                });
                            }
                            if let Some(catalog) = actors.get(&catalog_key) {
                                catalog.send(RuntimeCommand::RefreshSessions);
                            }
                        }
                        Err(message) => {
                            let _ = event_tx.send(RuntimeEvent::SessionsFailed {
                                generation: catalog_generation,
                                message,
                            });
                        }
                    }
                    continue;
                }
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
                    publish_session_status_if_changed(
                        &event_tx,
                        &mut published_statuses,
                        &selected,
                        session,
                        "Working",
                    );
                }
                if let RuntimeCommand::RenameSession { path, name, .. } = &command
                    && let Some((key, actor)) = actors.iter().find(|(key, _)| {
                        latest
                            .get(*key)
                            .and_then(|snapshot| snapshot.live_session.as_deref())
                            == Some(path.as_path())
                    })
                {
                    actor.send(RuntimeCommand::SetSessionName(name.clone()));
                    clock = clock.saturating_add(1);
                    last_touch.insert(key.clone(), clock);
                    continue;
                }
                let next = command_target(&command);
                if let Some((requested_key, project)) = next {
                    let _selection_timing = is_view_only_selection(&command)
                        .then(|| crate::performance::Timing::new("switch.runtime_route"));
                    let key = match &command {
                        RuntimeCommand::SelectSession { path, .. }
                        | RuntimeCommand::RestartSession { path, .. } => {
                            actor_paths.get(path).cloned().unwrap_or_else(|| {
                                actor_key_for_command(&command, &requested_key, &latest)
                            })
                        }
                        _ => requested_key,
                    };
                    clock = clock.saturating_add(1);
                    last_touch.insert(key.clone(), clock);
                    interacted.insert(key.clone());
                    let selection_changed = key != selected;
                    let view_only = is_view_only_selection(&command);
                    if selection_changed {
                        generation = generation.saturating_add(1);
                        selected = key.clone();
                        if !view_only {
                            let _ = event_tx.send(RuntimeEvent::SessionReset {
                                generation,
                                preserve_submission: false,
                            });
                        }
                    }
                    let resident_snapshot = latest.get(&key).cloned();
                    if let RuntimeCommand::SelectSession { path, .. }
                    | RuntimeCommand::RestartSession { path, .. } = &command
                    {
                        actor_paths.insert(path.clone(), key.clone());
                    }
                    let actor = actors.entry(key.clone()).or_insert_with(|| {
                        SessionRuntimeHandle::spawn(
                            project,
                            process_command.clone(),
                            false,
                            supervisor_thread.clone(),
                        )
                    });
                    if target_command_needs_actor_message(view_only, resident_snapshot.as_deref()) {
                        actor.send(command);
                    }
                    if let Some(mut snapshot) = resident_snapshot {
                        if view_only {
                            Arc::make_mut(&mut snapshot).transcript_changed_from = None;
                        }
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
                                system_notification_target: None,
                            });
                        }
                    }
                    if selection_changed && let Some(dialogs) = active_dialogs.get(&key) {
                        for request in dialogs {
                            let _ = event_tx.send(RuntimeEvent::ExtensionUi {
                                generation,
                                request: request.clone(),
                                system_notification_target: None,
                            });
                        }
                    }
                } else {
                    let target = if matches!(
                        &command,
                        RuntimeCommand::LoadSessions(_)
                            | RuntimeCommand::RefreshSessions
                            | RuntimeCommand::SetSessionCategory { .. }
                            | RuntimeCommand::RenameSession { .. }
                            | RuntimeCommand::MoveSession { .. }
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
            Err(mpsc::TryRecvError::Empty) => match activity_tracker.next_deadline() {
                Some(deadline) => {
                    thread::park_timeout(deadline.saturating_duration_since(Instant::now()))
                }
                None => thread::park(),
            },
            Err(mpsc::TryRecvError::Disconnected) => running = false,
        }
    }
    for actor in actors.values() {
        actor.send(RuntimeCommand::Shutdown);
    }
    for actor in actors.into_values() {
        actor.join();
    }
    let _ = event_tx.send(RuntimeEvent::Stopped);
}

fn initial_draft_command(id: String, project: PathBuf, session: Option<PathBuf>) -> RuntimeCommand {
    session.map_or(
        RuntimeCommand::ResumeDraft {
            id,
            project: project.clone(),
        },
        |path| RuntimeCommand::SelectSession { path, project },
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
        RuntimeCommand::SelectSession { path, project }
        | RuntimeCommand::RestartSession { path, project } => {
            Some((format!("session:{}", path.display()), project.clone()))
        }
        _ => None,
    }
}

fn is_view_only_selection(command: &RuntimeCommand) -> bool {
    matches!(command, RuntimeCommand::SelectSession { .. })
}

fn target_command_needs_actor_message(view_only: bool, resident: Option<&RuntimeSnapshot>) -> bool {
    !view_only
        || resident.is_none()
        || resident.is_some_and(|snapshot| !snapshot.connected && !snapshot.history_preview)
}

fn actor_key_for_command(
    command: &RuntimeCommand,
    requested_key: &str,
    latest: &HashMap<String, Arc<RuntimeSnapshot>>,
) -> String {
    let path = match command {
        RuntimeCommand::SelectSession { path, .. }
        | RuntimeCommand::RestartSession { path, .. } => path,
        _ => return requested_key.to_owned(),
    };
    latest
        .iter()
        .find(|(_, snapshot)| {
            snapshot.live_session.as_deref() == Some(path.as_path())
                || snapshot.selected_session.as_deref() == Some(path.as_path())
        })
        .map_or_else(|| requested_key.to_owned(), |(key, _)| key.clone())
}

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
    process_command: ProcessCommand,
    process: Option<RpcProcess>,
    login_process_only: bool,
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
    active_session: Option<PathBuf>,
    parked_snapshot: Option<RuntimeSnapshot>,
    deferred_prompt: Option<DeferredPrompt>,
    pending_session_controls: PendingSessionControls,
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
    result: Result<LoadedHistory, String>,
}

fn run(
    project: PathBuf,
    process_command: ProcessCommand,
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
        process_command,
        process: None,
        login_process_only: false,
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
        active_session: None,
        parked_snapshot: None,
        deferred_prompt: None,
        pending_session_controls: PendingSessionControls::default(),
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
        while let Some(item) = owner.process.as_mut().and_then(RpcProcess::try_next) {
            match owner.apply_process_item(item) {
                SnapshotChange::None => {}
                SnapshotChange::Streaming => {
                    let coalesced = stream_publish_due.is_some();
                    crate::performance::count_stream_event(coalesced);
                    if !coalesced {
                        stream_publish_due = Some(Instant::now() + STREAM_PUBLISH_INTERVAL);
                    }
                }
                SnapshotChange::Immediate => immediate_snapshot_change = true,
            }
        }
        if immediate_snapshot_change
            || stream_publish_due.is_some_and(|deadline| Instant::now() >= deadline)
        {
            owner.publish();
            stream_publish_due = None;
        }
        let now = Instant::now();
        let next_deadline = [stream_publish_due, owner.session_refresh_due]
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
        let _ = process.terminate();
    }
    let _ = owner.event_tx.send(RuntimeEvent::Stopped);
}

impl RuntimeOwner {
    fn start_process(&mut self, session: Option<PathBuf>) {
        self.process_generation = self.process_generation.saturating_add(1);
        let preserve_transcript = self.deferred_prompt.is_some()
            || (!self.pending_session_controls.is_empty() && self.snapshot.history_preview);
        let keep_preview = preserve_transcript && self.snapshot.history_preview;
        if let Some(mut old) = self.process.take() {
            let _ = old.terminate();
        }
        self.login_process_only = false;
        self.active_session = session.clone();
        self.parked_snapshot = None;
        self.startup_state_loaded = false;
        self.startup_history_loaded = false;
        self.pending_prompt_id = None;
        self.pending_prompt_item = None;
        self.transcript_changed_from = Some(0);
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
            preserve_submission: preserve_transcript,
        });
        self.publish();
        match RpcProcess::spawn_with_waker(
            &self.process_command,
            &self.project,
            session.as_deref(),
            thread::current(),
        ) {
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
            self.send(command);
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
                if let Some(state) = self.active_snapshot_mut().session.as_mut() {
                    state.session_name = Some(name.clone());
                }
                self.send(json!({"type": "set_session_name", "name": name}))
            }
            RuntimeCommand::RenameSession {
                path,
                project,
                name,
            } => match RpcProcess::rename_session(&self.process_command, &project, &path, &name) {
                Ok(()) => self.load_sessions(self.session_query.clone()),
                Err(message) => {
                    let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                        generation: self.session_generation,
                        message,
                    });
                }
            },
            RuntimeCommand::MoveSession { .. }
            | RuntimeCommand::StopSessionFamily { .. }
            | RuntimeCommand::DeleteSessionFamily { .. } => {}
            RuntimeCommand::NewSession { project, .. } => {
                self.project = project;
                self.start_process(None);
            }
            RuntimeCommand::ResumeDraft { project, .. } => self.resume_draft(project),
            RuntimeCommand::SelectSession { path, project } => self.select_history(path, project),
            RuntimeCommand::RestartSession { path, project } => {
                self.project = project;
                self.start_process(Some(path));
            }
            RuntimeCommand::RefreshSessionDocument { path, project } => {
                self.refresh_history(path, project)
            }
            RuntimeCommand::SetModel { provider, model_id } => self.set_model(provider, model_id),
            RuntimeCommand::Login(provider) => {
                if self.ensure_login_process() {
                    self.send(optional_string_command("login", "provider", provider));
                }
            }
            RuntimeCommand::SetThinking(level) => self.set_thinking(level),
            RuntimeCommand::SetPermissionLevel(level) => self.set_permission_level(level),
            RuntimeCommand::ExtensionResponse(response) => {
                if let Some(process) = self.process.as_mut()
                    && let Err(error) = process.send_extension_response(response)
                {
                    self.fail(error);
                }
            }
            RuntimeCommand::SetSessionCategory {
                path,
                in_review,
                archived,
            } => {
                if let Some(state) = &self.state
                    && let Err(error) = state.set_session_category(&path, in_review, archived)
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

    fn ensure_login_process(&mut self) -> bool {
        if self.process.is_some() {
            return true;
        }
        match RpcProcess::spawn_with_waker(
            &self.process_command,
            &self.project,
            None,
            thread::current(),
        ) {
            Ok(process) => {
                self.process = Some(process);
                self.login_process_only = true;
                true
            }
            Err(error) => {
                self.fail_login(error);
                false
            }
        }
    }

    fn finish_login_process(&mut self) {
        if self.login_process_only {
            if let Some(mut process) = self.process.take() {
                let _ = process.terminate();
            }
            self.login_process_only = false;
        }
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

    fn apply_process_item(&mut self, item: ProcessItem) -> SnapshotChange {
        match item {
            ProcessItem::Response(response) => {
                self.apply_response(response);
                SnapshotChange::None
            }
            ProcessItem::ExtensionUi(request) => {
                if request.workgraph_rpc().is_some() {
                    let rpc = crate::state::state_path()
                        .ok()
                        .and_then(|database| crate::workgraph_rpc::response(&request, &database));
                    let response = if let Some(rpc) = rpc {
                        if rpc.changed {
                            let _ = self.event_tx.send(RuntimeEvent::WorkGraphChanged {
                                project: self.project.clone(),
                            });
                        }
                        rpc.response
                    } else {
                        crate::protocol::ExtensionUiResponse::Value {
                            id: request.dialog_id().unwrap_or_default().to_owned(),
                            value: serde_json::json!({
                                "success": false,
                                "error": "work graph state is unavailable",
                            })
                            .to_string(),
                        }
                    };
                    if let Some(process) = self.process.as_mut()
                        && let Err(error) = process.send_extension_response(response)
                    {
                        self.fail(error);
                    }
                    return SnapshotChange::None;
                }
                let _ = self.event_tx.send(RuntimeEvent::ExtensionUi {
                    generation: self.process_generation,
                    request,
                    system_notification_target: None,
                });
                SnapshotChange::None
            }
            ProcessItem::Event(event) => {
                let event_type = event.get("type").and_then(Value::as_str);
                let settled = event_type == Some("agent_settled");
                let session_starting = event_type == Some("agent_start")
                    && self.active_session.is_none()
                    && self.parked_snapshot.is_none();
                let previewing = self.parked_snapshot.is_some();
                let previous_live_status =
                    previewing.then(|| session_badge_status(&self.active_snapshot().conversation));
                let (changed_from, snapshot_changed, live_status_changed) = {
                    let snapshot = self.active_snapshot_mut();
                    let (changed_from, conversation_state_changed) =
                        conversation_mut(snapshot).reduce_deferred_with_change(&event);
                    let context_changed = update_context_from_event(&mut snapshot.stats, &event);
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
                    self.send(command("get_state"));
                }
                if event_type == Some("agent_start") {
                    self.refresh_sessions();
                }
                if event_type == Some("session_info_changed") {
                    self.send(command("get_state"));
                    self.refresh_sessions();
                }
                if settled {
                    self.send(command("get_state"));
                    self.send(command("get_session_stats"));
                    self.refresh_sessions();
                }
                if !should_publish {
                    SnapshotChange::None
                } else if matches!(
                    event.get("type").and_then(Value::as_str),
                    Some("message_update" | "tool_execution_update")
                ) {
                    SnapshotChange::Streaming
                } else {
                    SnapshotChange::Immediate
                }
            }
            ProcessItem::Stderr(chunk) => {
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
            ProcessItem::Failure(error) => {
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
        let _timing = crate::performance::Timing::new("switch.select_document");
        self.history_generation = self.history_generation.saturating_add(1);
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
        self.refresh_history(path, project);
    }

    fn refresh_history(&mut self, path: PathBuf, project: PathBuf) {
        if self.active_session.as_deref() == Some(path.as_path()) && self.process.is_some() {
            return;
        }
        self.history_generation = self.history_generation.saturating_add(1);
        let generation = self.history_generation;
        let sender = self.history_tx.clone();
        let wake = thread::current();
        thread::Builder::new()
            .name("pi-gpui-history".into())
            .spawn(move || {
                let _timing = crate::performance::Timing::new("switch.load_history");
                let result = load_history(&path);
                let _ = sender.send(HistoryResult {
                    generation,
                    path,
                    project,
                    result,
                });
                wake.unpark();
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
        let history = match result.result {
            Ok(history) => history,
            Err(error) => {
                self.snapshot.status = "Could not load history".into();
                conversation_mut(&mut self.snapshot).push_local_error("History unavailable", error);
                self.publish();
                return;
            }
        };
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
            pending_question: history.pending_question,
            prefill_model,
            prefill_thinking_level: history.thinking_level,
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
            let startup_query = matches!(response.command.as_str(), "get_state" | "get_entries");
            let blocks_resume = self.deferred_prompt.is_some() && startup_query;
            let blocks_session_command_resume =
                !self.pending_session_controls.is_empty() && startup_query;
            if blocks_session_command_resume {
                let details = format!(
                    "{}: {}",
                    response.command,
                    response.error.unwrap_or_else(|| "command failed".into())
                );
                self.fail_session_control_resume("Command not sent", "Command not sent", details);
                return;
            }
            let login_failed_while_previewing =
                response.command == "login" && self.parked_snapshot.is_some();
            let snapshot = if login_failed_while_previewing {
                &mut self.snapshot
            } else {
                self.active_snapshot_mut()
            };
            conversation_mut(snapshot).push_local_error(
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
            if response.command == "login" {
                self.finish_login_process();
            }
            if self.parked_snapshot.is_none() || login_failed_while_previewing {
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
                    conversation_mut(snapshot).running = state.is_streaming;
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
            "get_entries" => {
                let entries = response
                    .data
                    .get("entries")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let messages = project_display_history(&entries);
                conversation_mut(self.active_snapshot_mut()).replace_history(&messages);
                self.startup_history_loaded = true;
            }
            "get_available_models" | "login" => {
                let cancelled = response.data.get("cancelled").and_then(Value::as_bool);
                let models = response
                    .data
                    .get("models")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default();
                self.active_snapshot_mut().models.clone_from(&models);
                if response.command == "login" {
                    let status = if cancelled == Some(true) {
                        "Ready"
                    } else {
                        "Provider added"
                    };
                    self.active_snapshot_mut().status = status.into();
                    if self.parked_snapshot.is_some() {
                        self.snapshot.models = models;
                        self.snapshot.status = status.into();
                    }
                }
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
            "get_session_stats" => {
                let running = self.active_snapshot().conversation.running;
                let previous = self.active_snapshot().stats.clone();
                self.active_snapshot_mut().stats =
                    stable_session_stats(&previous, response.data, running);
            }
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
        if matches!(response_command.as_str(), "get_state" | "get_entries") {
            self.maybe_send_deferred_prompt();
            self.maybe_send_pending_session_controls();
        }
        if response_command == "login" {
            self.finish_login_process();
        }
        if self.parked_snapshot.is_none() || response_command == "login" {
            self.publish();
        }
    }

    fn rollback_pending_prompt(&mut self) {
        if let Some(optimistic) = self.pending_prompt_item.take() {
            conversation_mut(self.active_snapshot_mut()).rollback_local_user(&optimistic);
        }
    }

    fn fail_login(&mut self, error: String) {
        let details = failure_details(&error);
        zlog::error!("Pi provider login failed: {details}");
        self.finish_login_process();
        let snapshot = if self.parked_snapshot.is_some() {
            &mut self.snapshot
        } else {
            self.active_snapshot_mut()
        };
        snapshot.status = "Failed".into();
        let conversation = conversation_mut(snapshot);
        conversation.diagnostics.push(details.clone());
        conversation.push_local_error_with_details(
            "Couldn’t add provider",
            failure_summary(&details),
            details,
        );
        self.publish();
    }

    fn fail(&mut self, error: String) {
        if self.login_process_only {
            self.fail_login(error);
            return;
        }
        let starting = !self.startup_state_loaded || !self.startup_history_loaded;
        let preserve_history = !self.pending_session_controls.is_empty()
            && self.snapshot.history_preview
            && self.parked_snapshot.is_some();
        let details = failure_details(&error);
        zlog::error!("Pi runtime failed: {details}");
        self.mark_outbox_failed(&details);
        self.pending_prompt_id = None;
        self.deferred_prompt = None;
        self.rollback_pending_prompt();
        if let Some(target) = self.pending_prompt_target.take() {
            self.emit_prompt_result(&target, false);
        }
        self.login_process_only = false;
        if preserve_history {
            self.fail_session_control_resume("Failed", "Couldn’t start Pi", details);
            return;
        }
        self.pending_session_controls = PendingSessionControls::default();
        if let Some(mut process) = self.process.take() {
            let _ = process.terminate();
        }
        let previewing = self.parked_snapshot.is_some();
        let snapshot = self.active_snapshot_mut();
        snapshot.connected = false;
        snapshot.status = "Failed".into();
        let conversation = conversation_mut(snapshot);
        conversation.diagnostics.push(details.clone());
        conversation.push_local_error_with_details(
            if starting {
                "Couldn’t start Pi"
            } else {
                "Pi stopped"
            },
            failure_summary(&details),
            details,
        );
        if previewing && let Some(snapshot) = self.parked_snapshot.take() {
            self.snapshot = snapshot;
        }
        self.publish();
    }

    fn mark_outbox_failed(&mut self, error: &str) {
        if let Some(id) = self.pending_outbox_id.take()
            && let Some(state) = &self.state
            && let Err(database_error) = state.fail_prompt(id, error)
        {
            zlog::error!("Failed to mark queued prompt {id} as failed: {database_error}");
        }
    }

    fn publish(&mut self) {
        crate::performance::count_snapshot();
        self.snapshot.permission_level =
            PermissionLevel::from_sandbox_disabled(self.process_command.sandbox_disabled);
        conversation_mut(self.active_snapshot_mut()).flush_live_projection();
        let active_snapshot = self.active_snapshot();
        let mut snapshot = self.snapshot.clone();
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

fn startup_commands() -> [Value; 7] {
    [
        json!({"type":"set_steering_mode","mode":"all"}),
        command("get_state"),
        command("get_entries"),
        command("get_session_stats"),
        command("get_available_models"),
        command("get_available_thinking_levels"),
        command("get_commands"),
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
mod tests {
    use std::{
        fs,
        os::unix::fs::symlink,
        time::{Duration, Instant, SystemTime},
    };

    use tempfile::tempdir;

    use super::*;

    fn test_event_channel() -> (SessionEventSender, mpsc::Receiver<RuntimeEvent>) {
        let (sender, receiver) = mpsc::channel();
        (
            SessionEventSender {
                sender,
                supervisor: thread::current(),
            },
            receiver,
        )
    }

    #[test]
    fn runtime_failure_summary_prefers_the_child_error() {
        let failure = "Pi closed stdout (exit code 1). Stderr:\n\
            Warning: settings were unavailable\n\
            Error: Failed to load extension codex-web-search-core.ts\n\
            Hint: Start without extensions";

        assert_eq!(
            failure_summary(failure),
            "Failed to load extension codex-web-search-core.ts"
        );
    }

    #[test]
    fn runtime_failure_details_remove_controls_and_bound_output() {
        let failure = format!("bad\u{1b}value {}", "x".repeat(MAX_FAILURE_DETAILS_CHARS));
        let details = failure_details(&failure);

        assert!(!details.contains('\u{1b}'));
        assert!(details.ends_with('…'));
        assert_eq!(details.chars().count(), MAX_FAILURE_DETAILS_CHARS + 1);
    }

    #[test]
    fn rpc_owned_paths_exclude_history_only_documents() {
        let active = PathBuf::from("/sessions/active.jsonl");
        let background = PathBuf::from("/sessions/background.jsonl");
        let history = PathBuf::from("/sessions/history.jsonl");
        let snapshot = |live: &PathBuf, selected: &PathBuf, history_preview| {
            Arc::new(RuntimeSnapshot {
                connected: true,
                live_session: Some(live.clone()),
                selected_session: Some(selected.clone()),
                history_preview,
                ..RuntimeSnapshot::default()
            })
        };
        let latest = HashMap::from([
            ("active".into(), snapshot(&active, &active, false)),
            ("preview".into(), snapshot(&background, &history, true)),
            ("history".into(), snapshot(&history, &history, true)),
        ]);

        assert_eq!(
            rpc_owned_session_paths(&latest),
            HashSet::from([active, background])
        );
    }

    #[test]
    fn dropping_runtime_waits_for_owned_pi_processes_to_handle_exit() -> Result<(), String> {
        let temp = tempdir().map_err(|error| error.to_string())?;
        let script = temp.path().join("fake-pi.sh");
        fs::write(&script, include_str!("../tests/fixtures/fake-pi.sh"))
            .map_err(|error| error.to_string())?;
        let marker = temp.path().join("terminated");
        let runtime = RuntimeHandle::spawn_with(
            temp.path().to_path_buf(),
            "shutdown-test".into(),
            None,
            ProcessCommand::test_script(
                &script,
                vec!["term-marker".into(), marker.to_string_lossy().into_owned()],
            ),
        );
        // RpcProcess permits up to 15 seconds for its readiness handshake. The
        // full test suite can also delay this supervisor under build-machine load.
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut connected = false;
        while Instant::now() < deadline && !connected {
            connected = matches!(
                runtime.try_recv(),
                Ok(RuntimeEvent::Snapshot { snapshot, .. }) if snapshot.connected
            );
            if !connected {
                thread::sleep(Duration::from_millis(5));
            }
        }
        assert!(connected, "owned Pi process did not start");

        drop(runtime);

        assert_eq!(
            fs::read_to_string(&marker).map_err(|error| error.to_string())?,
            "terminated\n",
            "runtime returned before Pi handled application exit"
        );
        Ok(())
    }

    fn owner_without_process(
        project: PathBuf,
    ) -> (
        RuntimeOwner,
        mpsc::Receiver<RuntimeEvent>,
        mpsc::Receiver<DiscoveryResult>,
    ) {
        let (event_tx, event_rx) = test_event_channel();
        let (discovery_tx, discovery_rx) = mpsc::channel();
        let (history_tx, _history_rx) = mpsc::channel();
        (
            RuntimeOwner {
                project: project.clone(),
                process_command: ProcessCommand {
                    program: PathBuf::from("/definitely/missing/pi-gpui-test-command"),
                    prefix_args: Vec::new(),
                    sandbox_disabled: false,
                },
                process: None,
                login_process_only: false,
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
                transcript_changed_from: None,
                event_tx,
                discovery_tx,
                history_tx,
                history_generation: 0,
                active_session: None,
                parked_snapshot: None,
                deferred_prompt: None,
                pending_session_controls: PendingSessionControls::default(),
                startup_state_loaded: false,
                startup_history_loaded: false,
                state: None,
                session_query: String::new(),
            },
            event_rx,
            discovery_rx,
        )
    }

    fn preview_history(owner: &mut RuntimeOwner, session: PathBuf, message: &str) {
        owner.snapshot.history_preview = true;
        owner.snapshot.selected_session = Some(session);
        conversation_mut(&mut owner.snapshot).replace_history(&[json!({
            "role": "user",
            "content": message,
            "timestamp": 1
        })]);
        owner.parked_snapshot = Some(RuntimeSnapshot::default());
    }

    #[test]
    fn permission_level_restarts_with_the_sandbox_flag() {
        let (mut owner, _events, _discovery) = owner_without_process(std::env::temp_dir());
        owner.snapshot.selected_session = Some(PathBuf::from("/session.jsonl"));
        let generation = owner.process_generation;

        owner.apply_command(RuntimeCommand::SetPermissionLevel(
            PermissionLevel::Sandboxed,
        ));
        assert_eq!(owner.process_generation, generation);
        assert!(!owner.process_command.sandbox_disabled);
        assert_eq!(owner.snapshot.permission_level, PermissionLevel::Sandboxed);

        owner.apply_command(RuntimeCommand::SetPermissionLevel(
            PermissionLevel::FullAccess,
        ));
        assert!(owner.process_generation > generation);
        assert!(owner.process_command.sandbox_disabled);
        assert_eq!(owner.snapshot.permission_level, PermissionLevel::FullAccess);
    }

    #[test]
    fn history_model_identity_survives_an_unavailable_catalog_entry() {
        let identity = ("opencode-go".into(), "kimi-k3".into());

        assert_eq!(
            SessionControlDefaults::history_model(&[], Some(&identity)),
            Some(Model {
                id: "kimi-k3".into(),
                name: "kimi-k3".into(),
                provider: "opencode-go".into(),
                context_window: 0,
                reasoning: false,
            })
        );
    }

    #[test]
    fn persisted_submitted_draft_selects_its_session() {
        let project = PathBuf::from("/project");
        let session = PathBuf::from("/sessions/submitted.jsonl");
        assert!(matches!(
            initial_draft_command("draft".into(), project.clone(), Some(session.clone())),
            RuntimeCommand::SelectSession { path, project: selected_project }
                if path == session && selected_project == project
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
    fn cloned_snapshots_share_conversation_storage() {
        let snapshot = RuntimeSnapshot::default();
        let cloned = snapshot.clone();

        assert!(Arc::ptr_eq(&snapshot.conversation, &cloned.conversation));
    }

    #[test]
    fn session_status_publication_deduplicates_but_tracks_session_changes() {
        let (events_tx, events_rx) = mpsc::channel();
        let (wake_tx, _wake_rx) = async_channel::bounded(1);
        let sender = UiEventSender {
            events: events_tx,
            wake: wake_tx,
        };
        let mut published = HashMap::new();

        publish_session_status_if_changed(&sender, &mut published, "target", None, "Working");
        publish_session_status_if_changed(&sender, &mut published, "target", None, "Working");
        publish_session_status_if_changed(
            &sender,
            &mut published,
            "target",
            Some(PathBuf::from("session.jsonl")),
            "Working",
        );

        assert_eq!(events_rx.try_iter().count(), 2);
    }

    #[test]
    fn session_documents_follow_interaction_archive_and_rpc_lifecycle() {
        let mut session = SessionSummary::from_cached(
            "one".into(),
            PathBuf::from("/sessions/one.jsonl"),
            PathBuf::from("/project"),
            "One".into(),
            "Question".into(),
            String::new(),
            None,
            SystemTime::now(),
            2,
            crate::sessions::UsageSummary::default(),
            false,
            false,
            String::new(),
        );

        assert!(!documents::session_document_is_live(&session, false, false));
        assert!(documents::session_document_is_live(&session, true, false));
        session.archived = true;
        assert!(!documents::session_document_is_live(&session, true, false));
        assert!(documents::session_document_is_live(&session, true, true));
        session.is_running = true;
        assert!(documents::session_document_is_live(&session, false, false));
    }

    #[test]
    fn running_stats_keep_the_last_meaningful_context_instead_of_flashing_zero() {
        let previous = json!({
            "contextUsage": {"tokens": 168_000, "contextWindow": 200_000, "percent": 84.0},
            "cost": 4
        });
        let next = json!({
            "contextUsage": {"tokens": 0, "contextWindow": 200_000, "percent": 0.0},
            "cost": 5
        });

        let merged = stable_session_stats(&previous, next.clone(), true);
        assert_eq!(merged["contextUsage"], previous["contextUsage"]);
        assert_eq!(merged["cost"], 5);
        assert_eq!(stable_session_stats(&previous, next.clone(), false), next);
    }

    #[test]
    fn first_running_turn_hides_empty_context_until_real_usage_arrives() {
        let next = json!({
            "contextUsage": {"tokens": 0, "contextWindow": 200_000, "percent": 0.0},
            "cost": 0
        });

        let merged = stable_session_stats(&Value::Null, next, true);
        assert!(merged.get("contextUsage").is_none());
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
                conversation: Arc::new(conversation),
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
                conversation: Arc::new(conversation),
                history_preview: true,
                ..RuntimeSnapshot::default()
            }),
            "Done"
        );
    }

    #[test]
    fn history_uses_latest_assistant_usage_for_context() {
        let messages = vec![json!({
            "role": "assistant",
            "provider": "test",
            "model": "model",
            "usage": {"input": 50, "output": 10, "cacheRead": 20, "cacheWrite": 20, "totalTokens": 100},
            "stopReason": "stop"
        })];
        let models = vec![Model {
            id: "model".into(),
            name: "Model".into(),
            provider: "test".into(),
            context_window: 200,
            reasoning: false,
        }];

        let stats = historical_context_stats(&messages, &models);
        assert_eq!(stats["contextUsage"]["tokens"], 100);
        assert_eq!(stats["contextUsage"]["contextWindow"], 200);
        assert_eq!(stats["contextUsage"]["percent"], 50.0);
    }

    #[test]
    fn streaming_usage_updates_context_before_agent_settles() {
        let mut stats = json!({
            "contextUsage": {"tokens": 40, "contextWindow": 200, "percent": 20.0}
        });
        assert!(update_context_from_event(
            &mut stats,
            &json!({
                "type": "message_update",
                "usage": {"input": 60, "output": 10, "cacheRead": 20, "cacheWrite": 10, "totalTokens": 100}
            }),
        ));
        assert_eq!(stats["contextUsage"]["tokens"], 100);
        assert_eq!(stats["contextUsage"]["percent"], 50.0);
        assert!(!update_context_from_event(
            &mut stats,
            &json!({
                "type": "message_update",
                "usage": {"totalTokens": 100}
            }),
        ));
    }

    #[test]
    fn completed_message_usage_updates_context_when_streaming_usage_is_unavailable() {
        let mut stats = json!({
            "contextUsage": {"tokens": 40, "contextWindow": 200, "percent": 20.0}
        });
        update_context_from_event(
            &mut stats,
            &json!({
                "type": "message_end",
                "message": {"usage": {"input": 50, "output": 10, "cacheRead": 20, "cacheWrite": 10}}
            }),
        );
        assert_eq!(stats["contextUsage"]["tokens"], 90);
        assert_eq!(stats["contextUsage"]["percent"], 45.0);
    }

    #[test]
    fn no_op_working_events_do_not_request_a_snapshot_publish() {
        let project = PathBuf::from("/project");
        let (mut owner, events, _discovery) = owner_without_process(project);
        owner.snapshot.status = "Working".into();
        conversation_mut(&mut owner.snapshot).running = true;

        let changed = owner.apply_process_item(ProcessItem::Event(json!({
            "type": "turn_start"
        })));

        assert_eq!(changed, SnapshotChange::None);
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn first_agent_action_refreshes_catalog_for_draft_promotion() {
        let (mut owner, events, _discovery) = owner_without_process(PathBuf::from("/project"));
        owner.owns_session_catalog = false;
        owner.active_session = Some(PathBuf::from("/sessions/new.jsonl"));

        owner.apply_process_item(ProcessItem::Event(json!({"type":"agent_start"})));

        assert!(matches!(
            events.try_recv(),
            Ok(RuntimeEvent::RefreshCatalog)
        ));
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
    fn notification_target_comes_from_the_emitting_runtime() {
        let snapshot = RuntimeSnapshot {
            project: PathBuf::from("/background-project"),
            live_session: Some(PathBuf::from("/background-session.jsonl")),
            selected_session: Some(PathBuf::from("/history-preview.jsonl")),
            ..RuntimeSnapshot::default()
        };

        assert_eq!(
            notification_target(&snapshot),
            Some((
                PathBuf::from("/background-session.jsonl"),
                PathBuf::from("/background-project"),
            ))
        );
    }

    #[test]
    fn interacted_session_document_hydrates_in_background_and_becomes_resident()
    -> Result<(), String> {
        let temp = tempdir().map_err(|error| error.to_string())?;
        let path = temp.path().join("session.jsonl");
        fs::write(
            &path,
            format!(
                "{{\"type\":\"session\",\"id\":\"one\",\"cwd\":{},\"timestamp\":\"2026-08-19T00:00:00Z\"}}\n{{\"type\":\"message\",\"id\":\"message\",\"parentId\":null,\"message\":{{\"role\":\"user\",\"content\":\"resident\"}}}}\n",
                serde_json::to_string(temp.path()).map_err(|error| error.to_string())?
            ),
        )
        .map_err(|error| error.to_string())?;
        let modified = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .map_err(|error| error.to_string())?;
        let session = SessionSummary::from_cached(
            "one".into(),
            path.clone(),
            temp.path().to_path_buf(),
            "One".into(),
            "resident".into(),
            String::new(),
            None,
            modified,
            1,
            crate::sessions::UsageSummary::default(),
            false,
            false,
            String::new(),
        );
        let key = format!("session:{}", path.display());
        let mut actors = HashMap::new();
        let mut documents = HashMap::new();
        let mut touches = HashMap::new();
        let mut revisions = HashMap::new();
        let mut paths = HashMap::new();

        reconcile_live_session_documents(
            std::slice::from_ref(&session),
            &HashSet::from([key.clone()]),
            "other",
            &mut actors,
            &mut documents,
            &mut touches,
            &mut revisions,
            &mut paths,
            &ProcessCommand::default(),
            &thread::current(),
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut resident = None;
        while Instant::now() < deadline {
            let Some(actor) = actors.get(&key) else {
                return Err("document actor was not retained".into());
            };
            while let Ok(event) = actor.events.try_recv() {
                if let RuntimeEvent::Snapshot { snapshot, .. } = event
                    && snapshot.selected_session.as_deref() == Some(path.as_path())
                    && snapshot
                        .conversation
                        .items
                        .iter()
                        .any(|item| item.text == "resident")
                {
                    resident = Some(snapshot);
                    break;
                }
            }
            if resident.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let resident = resident.ok_or_else(|| "document did not hydrate".to_owned())?;
        documents.insert(key.clone(), resident);
        let mut archived = session;
        archived.archived = true;
        reconcile_live_session_documents(
            &[archived],
            &HashSet::from([key.clone()]),
            "other",
            &mut actors,
            &mut documents,
            &mut touches,
            &mut revisions,
            &mut paths,
            &ProcessCommand::default(),
            &thread::current(),
        );

        assert!(!actors.contains_key(&key));
        assert!(!documents.contains_key(&key));
        assert!(!paths.contains_key(&path));
        assert!(!revisions.contains_key(&path));
        Ok(())
    }

    #[test]
    fn selecting_a_resident_document_does_not_reload_or_message_its_actor() {
        let history = RuntimeSnapshot {
            connected: false,
            history_preview: true,
            ..RuntimeSnapshot::default()
        };
        let disconnected = RuntimeSnapshot::default();

        assert!(!target_command_needs_actor_message(true, Some(&history)));
        assert!(target_command_needs_actor_message(
            true,
            Some(&disconnected)
        ));
        assert!(target_command_needs_actor_message(true, None));
        assert!(target_command_needs_actor_message(false, Some(&history)));
    }

    #[test]
    fn saved_path_reuses_the_actor_that_started_as_a_draft() {
        let path = PathBuf::from("/sessions/one.jsonl");
        let command = RuntimeCommand::SelectSession {
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

        assert!(is_view_only_selection(&command));
        assert_eq!(
            actor_key_for_command(&command, &format!("session:{}", path.display()), &latest,),
            "draft:one"
        );

        let restart = RuntimeCommand::RestartSession {
            path: path.clone(),
            project: PathBuf::from("/project"),
        };
        assert!(!is_view_only_selection(&restart));
        assert_eq!(
            actor_key_for_command(&restart, &format!("session:{}", path.display()), &latest,),
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
        conversation_mut(&mut owner.snapshot).reduce(&json!({"type":"agent_start"}));
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
        let session = temp.path().join("session.jsonl");
        let (mut owner, _events, _discovery) = owner_without_process(temp.path().to_path_buf());
        owner.process_command = ProcessCommand::test_script(&script, vec!["quiet".into()]);
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
        let process = RpcProcess::spawn(
            &ProcessCommand::test_script(&script, vec!["quiet".into()]),
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
        let (mut owner, _events, _discovery) =
            owner_without_process(old_project.path().to_path_buf());
        owner.process_command = ProcessCommand::test_script(&script, vec!["quiet".into()]);

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
    fn filesystem_changes_coalesce_into_one_delayed_scan() {
        let (mut owner, _events, _discovery) = owner_without_process(std::env::temp_dir());
        owner.schedule_session_refresh();
        let due = owner.session_refresh_due.expect("filesystem refresh");
        owner.schedule_session_refresh();
        assert_eq!(owner.session_refresh_due, Some(due));

        owner.poll_deferred_session_refresh(due - Duration::from_millis(1));
        assert_eq!(owner.session_generation, 0);
        owner.poll_deferred_session_refresh(due);
        assert_eq!(owner.session_generation, 1);
        assert!(owner.session_discovery_in_flight);
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
            "Implementation child".into(),
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
    fn partial_catalog_refresh_keeps_omitted_running_children_in_the_catalog()
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
            RuntimeEvent::Sessions { all_sessions, .. }
                if all_sessions.iter().any(|session| session.id == "child")
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
            context_window: 200_000,
            reasoning: true,
        };
        let mut controls = SessionControlDefaults::default();
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
        controls.apply(&mut ready, true);

        let mut starting = RuntimeSnapshot::default();
        controls.apply(&mut starting, true);

        assert_eq!(starting.prefill_model, Some(model.clone()));
        assert_eq!(starting.prefill_thinking_level.as_deref(), Some("high"));
        assert_eq!(starting.models, vec![model]);
        assert_eq!(starting.thinking_levels, vec!["off", "high"]);
        assert!(starting.session.is_none());
    }

    #[test]
    fn history_identity_overrides_draft_defaults_without_changing_them() {
        let sol = Model {
            id: "gpt-5.6-sol".into(),
            name: "Sol".into(),
            provider: "openai-codex".into(),
            context_window: 200_000,
            reasoning: true,
        };
        let luna = Model {
            id: "gpt-5.6-luna".into(),
            name: "Luna".into(),
            provider: "openai-codex".into(),
            context_window: 200_000,
            reasoning: true,
        };
        let mut defaults = SessionControlDefaults::default();
        let mut live_sol = RuntimeSnapshot {
            session: serde_json::from_value(json!({
                "model": sol,
                "thinkingLevel": "high",
                "isStreaming": false,
                "isCompacting": false,
                "sessionFile": "/sol",
                "sessionId": "sol",
                "autoCompactionEnabled": true,
                "messageCount": 0,
                "pendingMessageCount": 0
            }))
            .ok(),
            models: vec![sol.clone(), luna.clone()],
            thinking_levels: vec!["medium".into(), "high".into()],
            ..RuntimeSnapshot::default()
        };
        defaults.apply(&mut live_sol, true);

        let mut luna_history = RuntimeSnapshot {
            prefill_model: Some(luna.clone()),
            prefill_thinking_level: Some("medium".into()),
            history_preview: true,
            ..RuntimeSnapshot::default()
        };
        defaults.apply(&mut luna_history, false);

        let history_identity = luna_history.session_identity();
        assert_eq!(history_identity.provider, Some("openai-codex"));
        assert_eq!(history_identity.model, Some(&luna));
        assert_eq!(history_identity.effort, Some("medium"));

        let mut empty_draft = RuntimeSnapshot::default();
        defaults.apply(&mut empty_draft, true);
        let draft_identity = empty_draft.session_identity();
        assert_eq!(draft_identity.model, Some(&sol));
        assert_eq!(draft_identity.effort, Some("high"));
    }

    #[test]
    fn viewing_a_subagent_does_not_change_new_session_defaults() {
        let sol = Model {
            id: "gpt-5.6-sol".into(),
            name: "Sol".into(),
            provider: "openai-codex".into(),
            context_window: 200_000,
            reasoning: true,
        };
        let luna = Model {
            id: "gpt-5.6-luna".into(),
            name: "Luna".into(),
            provider: "openai-codex".into(),
            context_window: 200_000,
            reasoning: true,
        };
        let session_state = |path: &str, model: Model| {
            serde_json::from_value(json!({
                "model": model,
                "thinkingLevel": "high",
                "isStreaming": false,
                "isCompacting": false,
                "sessionFile": path,
                "sessionId": path,
                "autoCompactionEnabled": true,
                "messageCount": 0,
                "pendingMessageCount": 0
            }))
            .ok()
        };
        let mut defaults = SessionControlDefaults::default();
        let mut root = RuntimeSnapshot {
            session: session_state("/sessions/root.jsonl", sol.clone()),
            models: vec![sol.clone(), luna.clone()],
            thinking_levels: vec!["medium".into(), "high".into()],
            ..RuntimeSnapshot::default()
        };
        defaults.apply(&mut root, true);

        // The user views a subagent running a different model; the supervisor passes
        // adopt_identity=false for descendant sessions.
        let mut subagent = RuntimeSnapshot {
            live_session: Some(PathBuf::from("/sessions/child.jsonl")),
            session: session_state("/sessions/child.jsonl", luna.clone()),
            models: vec![sol.clone(), luna.clone()],
            thinking_levels: vec!["medium".into(), "high".into()],
            ..RuntimeSnapshot::default()
        };
        defaults.apply(&mut subagent, false);

        let mut new_draft = RuntimeSnapshot::default();
        defaults.apply(&mut new_draft, true);
        let identity = new_draft.session_identity();
        assert_eq!(identity.model, Some(&sol));
        assert_eq!(identity.effort, Some("high"));
    }

    #[test]
    fn startup_delivers_all_composer_steers_at_one_turn_boundary() {
        assert_eq!(
            startup_commands()[0],
            json!({"type":"set_steering_mode","mode":"all"})
        );
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
                context_window: 0,
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
        conversation_mut(&mut snapshot).reduce(&json!({"type": "agent_start"}));
        conversation_mut(&mut snapshot).reduce(&json!({
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
        assert_eq!(*snapshot.conversation, ConversationState::default());
    }

    #[test]
    fn login_from_history_uses_a_temporary_process_without_clearing_transcript()
    -> Result<(), String> {
        let temp = tempdir().map_err(|error| error.to_string())?;
        let script = temp.path().join("fake-pi.sh");
        fs::write(&script, include_str!("../tests/fixtures/fake-pi.sh"))
            .map_err(|error| error.to_string())?;
        let (mut owner, _events, _discovery) = owner_without_process(temp.path().to_path_buf());
        owner.process_command = ProcessCommand::test_script(&script, vec!["normal".into()]);
        owner.snapshot.history_preview = true;
        owner.snapshot.selected_session = Some(temp.path().join("history.jsonl"));
        conversation_mut(&mut owner.snapshot).replace_history(&[json!({
            "role": "user",
            "content": "keep this transcript",
            "timestamp": 1
        })]);
        owner.parked_snapshot = Some(RuntimeSnapshot::default());
        let transcript = owner.snapshot.conversation.items.clone();

        owner.apply_command(RuntimeCommand::Login(None));

        assert!(owner.login_process_only);
        assert!(owner.process.is_some());
        assert_eq!(owner.snapshot.conversation.items, transcript);
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && owner.process.is_some() {
            if let Some(item) = owner.process.as_mut().and_then(RpcProcess::try_next) {
                let _ = owner.apply_process_item(item);
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }
        assert!(owner.process.is_none());
        assert!(!owner.login_process_only);
        assert_eq!(owner.snapshot.conversation.items, transcript);
        assert_eq!(owner.snapshot.status, "Provider added");
        Ok(())
    }

    #[test]
    fn model_change_from_history_reconnects_without_hiding_history() -> Result<(), String> {
        let temp = tempdir().map_err(|error| error.to_string())?;
        let script = temp.path().join("fake-pi.sh");
        fs::write(&script, include_str!("../tests/fixtures/fake-pi.sh"))
            .map_err(|error| error.to_string())?;
        let session = temp.path().join("history.jsonl");
        let (mut owner, _events, _discovery) = owner_without_process(temp.path().to_path_buf());
        owner.process_command =
            ProcessCommand::test_script(&script, vec!["history-control".into()]);
        preview_history(&mut owner, session.clone(), "preserved history");

        owner.apply_command(RuntimeCommand::SetModel {
            provider: "new-provider".into(),
            model_id: "new-model".into(),
        });

        assert!(owner.process.is_some());
        assert!(owner.snapshot.history_preview);
        assert_eq!(
            owner.snapshot.conversation.items[0].text,
            "preserved history"
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            while let Some(item) = owner.process.as_mut().and_then(RpcProcess::try_next) {
                owner.apply_process_item(item);
            }
            if owner.pending_session_controls.is_empty()
                && owner
                    .snapshot
                    .session
                    .as_ref()
                    .and_then(|state| state.model.as_ref())
                    .is_some_and(|model| model.id == "new-model")
            {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }

        assert!(!owner.snapshot.history_preview);
        assert_eq!(owner.snapshot.selected_session, Some(session));
        assert!(
            owner.snapshot.conversation.items.iter().any(|item| {
                item.kind == TranscriptKind::User && item.text == "preserved history"
            })
        );
        assert_eq!(
            owner
                .snapshot
                .session
                .as_ref()
                .and_then(|state| state.model.as_ref())
                .map(|model| (model.provider.as_str(), model.id.as_str())),
            Some(("new-provider", "new-model"))
        );
        Ok(())
    }

    #[test]
    fn failed_model_reconnect_keeps_the_loaded_history() {
        let project = std::env::temp_dir();
        let session = project.join("history.jsonl");
        let (mut owner, _events, _discovery) = owner_without_process(project);
        owner.process_command = ProcessCommand {
            program: PathBuf::from("/definitely/missing/pi-gpui-test-command"),
            prefix_args: Vec::new(),
            sandbox_disabled: false,
        };
        preview_history(&mut owner, session.clone(), "keep this history");

        owner.apply_command(RuntimeCommand::SetModel {
            provider: "provider".into(),
            model_id: "model".into(),
        });

        assert!(owner.process.is_none());
        assert!(owner.snapshot.history_preview);
        assert_eq!(owner.snapshot.selected_session, Some(session));
        assert_eq!(
            owner.snapshot.conversation.items[0].text,
            "keep this history"
        );
        assert!(owner.snapshot.conversation.items.iter().any(|item| {
            item.kind == TranscriptKind::Error && item.label == "Couldn’t start Pi"
        }));
    }

    #[test]
    fn failed_resume_publishes_no_state_from_the_previous_process() {
        let (event_tx, event_rx) = test_event_channel();
        let (discovery_tx, _discovery_rx) = mpsc::channel();
        let (history_tx, _history_rx) = mpsc::channel();
        let mut owner = RuntimeOwner {
            project: std::env::temp_dir(),
            process_command: ProcessCommand {
                program: PathBuf::from("/definitely/missing/pi-gpui-test-command"),
                prefix_args: Vec::new(),
                sandbox_disabled: false,
            },
            process: None,
            login_process_only: false,
            snapshot: RuntimeSnapshot {
                connected: true,
                status: "old".into(),
                selected_session: Some(PathBuf::from("/old")),
                models: vec![Model {
                    id: "old".into(),
                    name: "Old".into(),
                    provider: "test".into(),
                    context_window: 0,
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
            transcript_changed_from: None,
            event_tx,
            discovery_tx,
            history_tx,
            history_generation: 0,
            active_session: Some(PathBuf::from("/old")),
            parked_snapshot: None,
            deferred_prompt: None,
            pending_session_controls: PendingSessionControls::default(),
            startup_state_loaded: false,
            startup_history_loaded: false,
            state: None,
            session_query: String::new(),
        };
        conversation_mut(&mut owner.snapshot).reduce(&json!({
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
        let error = latest
            .conversation
            .items
            .iter()
            .find(|item| item.kind == TranscriptKind::Error)
            .expect("failed process should publish a visible error");
        assert_eq!(error.label, "Couldn’t start Pi");
        assert!(error.text.contains("definitely/missing"));
        assert!(error.tool_output.contains("definitely/missing"));
        assert!(
            latest
                .conversation
                .diagnostics
                .iter()
                .any(|item| item.contains("definitely/missing"))
        );
    }

    #[test]
    fn failed_start_marks_the_deferred_prompt_failed() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let database = temp.path().join("gui-state.sqlite3");
        let (mut owner, _events, _discovery) = owner_without_process(temp.path().to_path_buf());
        owner.process_command = ProcessCommand {
            program: PathBuf::from("/definitely/missing/pi-gpui-test-command"),
            prefix_args: Vec::new(),
            sandbox_disabled: false,
        };
        owner.state = Some(StateStore::open_at(&database)?);

        owner.send_prompt(
            "draft:failed-start".into(),
            PromptMode::Normal,
            "hello".into(),
            Vec::new(),
            false,
        );

        assert!(
            owner
                .state
                .as_ref()
                .expect("state")
                .queued_prompts()?
                .is_empty()
        );
        assert!(owner.pending_outbox_id.is_none());
        let connection = rusqlite::Connection::open(database)?;
        let (state, error) = connection.query_row(
            "SELECT state, error FROM outbox ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        assert_eq!(state, "failed");
        assert!(error.contains("definitely/missing"));
        Ok(())
    }

    #[test]
    fn selecting_from_an_idle_session_does_not_start_pi() {
        let old_project = PathBuf::from("/old-project");
        let new_project = PathBuf::from("/new-project");
        let old_path = PathBuf::from("/old-session.jsonl");
        let new_path = PathBuf::from("/new-session.jsonl");
        let (mut owner, events, _discovery) = owner_without_process(old_project);
        owner.active_session = Some(old_path.clone());
        owner.snapshot.selected_session = Some(old_path.clone());
        owner.process_generation = 4;

        owner.select_history(new_path, new_project);

        assert_eq!(owner.process_generation, 4);
        assert_eq!(owner.active_session, Some(old_path));
        assert!(owner.process.is_none());
        assert!(
            events
                .try_iter()
                .all(|event| !matches!(event, RuntimeEvent::SessionReset { .. }))
        );
    }

    #[test]
    fn history_preview_keeps_running_pi_until_a_prompt_resumes_the_session() -> Result<(), String> {
        let temp = tempdir().map_err(|error| error.to_string())?;
        let script = temp.path().join("fake-pi.sh");
        fs::write(&script, include_str!("../tests/fixtures/fake-pi.sh"))
            .map_err(|error| error.to_string())?;
        let process_command = ProcessCommand::test_script(&script, vec!["quiet".into()]);
        let process = RpcProcess::spawn(&process_command, temp.path(), None)?;
        let (event_tx, event_rx) = test_event_channel();
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
            login_process_only: false,
            snapshot: RuntimeSnapshot {
                connected: true,
                status: "Working".into(),
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
            transcript_changed_from: None,
            event_tx,
            discovery_tx,
            history_tx,
            history_generation: 1,
            active_session: Some(old_path.clone()),
            parked_snapshot: None,
            deferred_prompt: None,
            pending_session_controls: PendingSessionControls::default(),
            startup_state_loaded: false,
            startup_history_loaded: false,
            state: Some(StateStore::open_at(&temp.path().join("gui-state.sqlite3"))?),
            session_query: String::new(),
        };
        conversation_mut(&mut owner.snapshot).running = true;

        owner.select_history(old_path.clone(), old_project);
        owner.apply_history(HistoryResult {
            generation: 1,
            path: new_path.clone(),
            project: new_project.clone(),
            result: Ok(crate::sessions::LoadedHistory {
                messages: vec![json!({"role":"user","content":"previewed"})],
                model: None,
                thinking_level: None,
                pending_question: None,
            }),
        });
        assert!(!owner.snapshot.history_preview);
        owner.apply_history(HistoryResult {
            generation: 2,
            path: new_path.clone(),
            project: new_project.clone(),
            result: Ok(crate::sessions::LoadedHistory {
                messages: vec![json!({"role":"user","content":"previewed"})],
                model: None,
                thinking_level: None,
                pending_question: Some(ExtensionUiRequest::Input {
                    id: "restored-question:one".into(),
                    title: "Continue?".into(),
                    placeholder: None,
                    timeout: None,
                }),
            }),
        });

        assert!(owner.process.is_some());
        assert!(owner.snapshot.history_preview);
        assert!(owner.snapshot.pending_question.is_some());
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
            true,
        );
        assert!(owner.snapshot.history_preview);
        assert_eq!(owner.snapshot.conversation.items[0].text, "previewed");
        assert!(owner.snapshot.pending_question.is_none());
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
                        && snapshot.pending_question.is_none()
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

        let (event_tx, event_rx) = test_event_channel();
        let (discovery_tx, _discovery_rx) = mpsc::channel();
        let (history_tx, history_rx) = mpsc::channel();
        let mut owner = RuntimeOwner {
            project: temp.path().to_path_buf(),
            process_command: ProcessCommand::default(),
            process: None,
            login_process_only: false,
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
            transcript_changed_from: None,
            event_tx,
            discovery_tx,
            history_tx,
            history_generation: 0,
            active_session: Some(active_path.clone()),
            parked_snapshot: None,
            deferred_prompt: None,
            pending_session_controls: PendingSessionControls::default(),
            startup_state_loaded: true,
            startup_history_loaded: true,
            state: None,
            session_query: String::new(),
        };
        conversation_mut(&mut owner.snapshot).reduce(&json!({"type":"agent_start"}));

        owner.select_history(history_path.clone(), history_project.clone());
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

        assert_eq!(
            owner.apply_process_item(ProcessItem::Event(json!({
                "type": "message_start",
                "message": {
                    "role": "assistant",
                    "content": [{"type": "text", "text": "active output"}]
                }
            }))),
            SnapshotChange::None
        );

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
        assert_eq!(changed, SnapshotChange::Immediate);
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

        owner.select_history(active_path.clone(), temp.path().to_path_buf());
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
