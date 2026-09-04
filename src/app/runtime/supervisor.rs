//! Runtime thread handles and multi-session command routing.

use super::*;

mod commands;
mod events;
mod family_commands;

pub(crate) struct RuntimeHandle {
    commands: mpsc::Sender<RuntimeCommand>,
    events: mpsc::Receiver<RuntimeEvent>,
    wake: async_channel::Receiver<()>,
    thread: thread::Thread,
    join: Option<thread::JoinHandle<()>>,
}

#[derive(Clone)]
pub(super) struct UiEventSender {
    pub(super) events: mpsc::Sender<RuntimeEvent>,
    pub(super) wake: async_channel::Sender<()>,
}

fn delete_session_files(paths: &[PathBuf]) -> Result<Vec<(PathBuf, String)>, String> {
    if paths
        .first()
        .is_some_and(|path| agents::is_external_session(path))
    {
        for path in paths.iter().rev() {
            agents::delete_external_session(path)
                .ok_or_else(|| "session family mixes backend locators".to_owned())??;
        }
        Ok(Vec::new())
    } else {
        sessions::delete_family(paths)
    }
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
        app_proxy: Option<String>,
    ) -> Self {
        let command = AgentLaunchConfig {
            app_proxy,
            session_locator_root: crate::app::paths::data_dir()
                .ok()
                .map(|root| root.join("session-locators")),
            ..AgentLaunchConfig::default()
        };
        Self::spawn_with_configuration_refresh(project, draft_id, initial_session, command, true)
    }

    #[cfg(test)]
    pub(crate) fn spawn_with(
        project: PathBuf,
        draft_id: String,
        initial_session: Option<PathBuf>,
        process_command: AgentLaunchConfig,
    ) -> Self {
        Self::spawn_with_configuration_refresh(
            project,
            draft_id,
            initial_session,
            process_command,
            false,
        )
    }

    fn spawn_with_configuration_refresh(
        project: PathBuf,
        draft_id: String,
        initial_session: Option<PathBuf>,
        process_command: AgentLaunchConfig,
        refresh_configuration: bool,
    ) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (events_tx, events) = mpsc::channel();
        let (wake_tx, wake) = async_channel::bounded(1);
        let event_tx = UiEventSender {
            events: events_tx,
            wake: wake_tx,
        };
        let handle = thread::Builder::new()
            .name("farcaster-supervisor".into())
            .spawn(move || {
                run_supervisor(
                    project,
                    draft_id,
                    initial_session,
                    process_command,
                    command_rx,
                    event_tx,
                    refresh_configuration,
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

#[derive(Clone)]
pub(super) struct SessionEventSender {
    pub(super) sender: mpsc::Sender<RuntimeEvent>,
    pub(super) supervisor: thread::Thread,
}

impl SessionEventSender {
    pub(super) fn send(&self, event: RuntimeEvent) -> Result<(), ()> {
        self.sender.send(event).map_err(|_| ())?;
        self.supervisor.unpark();
        Ok(())
    }
}

pub(super) struct SessionRuntimeHandle {
    commands: mpsc::Sender<RuntimeCommand>,
    pub(super) events: mpsc::Receiver<RuntimeEvent>,
    thread: thread::Thread,
    join: thread::JoinHandle<()>,
}

impl SessionRuntimeHandle {
    pub(super) fn spawn(
        project: PathBuf,
        process_command: AgentLaunchConfig,
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
            .name("farcaster-session".into())
            .spawn(move || run(project, process_command, command_rx, event_tx, load_catalog))
            .expect("start Pi session runtime");
        Self {
            commands,
            events,
            thread: handle.thread().clone(),
            join: handle,
        }
    }

    pub(super) fn send(&self, command: RuntimeCommand) {
        if self.commands.send(command).is_ok() {
            self.thread.unpark();
        }
    }

    fn join(self) {
        let _ = self.join.join();
    }
}

pub(super) fn publish_session_status_if_changed(
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

pub(super) fn rpc_owned_session_paths(
    latest: &HashMap<String, Arc<RuntimeSnapshot>>,
) -> HashSet<PathBuf> {
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

pub(super) fn changed_external_documents(
    latest: &HashMap<String, Arc<RuntimeSnapshot>>,
    paths: &[PathBuf],
) -> Vec<(String, PathBuf, PathBuf)> {
    latest
        .iter()
        .filter_map(|(key, snapshot)| {
            let path = snapshot.selected_session.as_ref()?;
            if !snapshot.history_preview
                || !paths.iter().any(|candidate| {
                    candidate == path
                        || crate::sessions::normalize_session_path(candidate).as_path()
                            == path.as_path()
                })
            {
                None
            } else {
                Some((key.clone(), path.clone(), snapshot.project.clone()))
            }
        })
        .collect()
}

fn cache_configuration_catalog(
    entries: &mut Vec<crate::app::infrastructure::persistence::CachedConfigurationCatalog>,
    harness: String,
    project: PathBuf,
    catalog: crate::agents::ConfigurationCatalog,
) -> bool {
    if let Some(entry) = entries
        .iter_mut()
        .find(|entry| entry.harness == harness && entry.project == project)
    {
        if entry.catalog == catalog {
            return false;
        }
        entry.catalog = catalog;
        return true;
    }
    entries.push(
        crate::app::infrastructure::persistence::CachedConfigurationCatalog {
            harness,
            project,
            catalog,
        },
    );
    true
}

fn adopts_selected_configuration(snapshot: &RuntimeSnapshot, sessions: &[SessionSummary]) -> bool {
    let session_path = snapshot
        .live_session
        .as_ref()
        .or(snapshot.selected_session.as_ref());
    !session_path.is_some_and(|path| crate::sessions::is_subagent_path(sessions, path))
}

fn update_selected_configuration(
    configurations: &mut HarnessConfigurationStore,
    snapshot: &RuntimeSnapshot,
    command: &RuntimeCommand,
) -> bool {
    match command {
        RuntimeCommand::SetModel(model) => {
            configurations.set_model(&snapshot.harness, model.clone())
        }
        RuntimeCommand::SetThinking(effort) => {
            configurations.set_effort(&snapshot.harness, effort.clone())
        }
        _ => false,
    }
}

fn persist_configurations(state: Option<&StateStore>, configurations: &HarnessConfigurationStore) {
    if let Some(state) = state {
        let _ = state.save_session_control_defaults(&configurations.cached());
    }
}

fn send_configured_command(
    actor: &SessionRuntimeHandle,
    command: RuntimeCommand,
    configurations: &HarnessConfigurationStore,
) {
    let selection = match &command {
        RuntimeCommand::NewSession { harness, .. }
        | RuntimeCommand::ResumeDraft { harness, .. } => Some((
            configurations.model(harness),
            configurations.effort(harness),
        )),
        _ => None,
    };
    actor.send(command);
    let Some((model, effort)) = selection else {
        return;
    };
    if let Some(model) = model {
        actor.send(RuntimeCommand::SetModel(model.clone()));
    }
    if let Some(effort) = effort {
        actor.send(RuntimeCommand::SetThinking(effort.to_owned()));
    }
}

fn refresh_configuration_catalogs(
    project: PathBuf,
    process_command: AgentLaunchConfig,
    supervisor: thread::Thread,
    sender: mpsc::Sender<(
        String,
        PathBuf,
        Result<crate::agents::ConfigurationCatalog, String>,
    )>,
) {
    for backend in agents::backend_statuses()
        .into_iter()
        .filter(|backend| backend.available)
    {
        let project = project.clone();
        let process_command = process_command.clone();
        let supervisor = supervisor.clone();
        let sender = sender.clone();
        let harness = backend.id;
        let _ = thread::Builder::new()
            .name(format!("farcaster-{harness}-catalog"))
            .spawn(move || {
                let result =
                    agents::load_configuration_catalog(&process_command, &harness, &project);
                let _ = sender.send((harness, project, result));
                supervisor.unpark();
            });
    }
}

struct Supervisor {
    process_command: AgentLaunchConfig,
    command_rx: mpsc::Receiver<RuntimeCommand>,
    event_tx: UiEventSender,
    supervisor_thread: thread::Thread,
    catalog_key: String,
    actors: HashMap<String, SessionRuntimeHandle>,
    selected: String,
    generation: u64,
    latest: HashMap<String, Arc<RuntimeSnapshot>>,
    catalog_sessions: Vec<SessionSummary>,
    catalog_generation: u64,
    activity_tracker: ExternalActivityTracker,
    actor_paths: HashMap<PathBuf, String>,
    interacted: HashSet<String>,
    document_revisions: HashMap<PathBuf, (SystemTime, usize)>,
    pending_extensions: HashMap<String, Vec<crate::protocol::ExtensionUiRequest>>,
    active_dialogs: HashMap<String, Vec<crate::protocol::ExtensionUiRequest>>,
    needs_input: HashSet<String>,
    clock: u64,
    last_touch: HashMap<String, u64>,
    configurations: HarnessConfigurationStore,
    catalog_state: Option<StateStore>,
    configuration_catalogs:
        Vec<crate::app::infrastructure::persistence::CachedConfigurationCatalog>,
    configuration_rx: mpsc::Receiver<(
        String,
        PathBuf,
        Result<crate::agents::ConfigurationCatalog, String>,
    )>,
    published_statuses: HashMap<String, (Option<PathBuf>, String)>,
}

fn run_supervisor(
    project: PathBuf,
    draft_id: String,
    initial_session: Option<PathBuf>,
    process_command: AgentLaunchConfig,
    command_rx: mpsc::Receiver<RuntimeCommand>,
    event_tx: UiEventSender,
    refresh_configuration: bool,
) {
    Supervisor::new(
        project,
        draft_id,
        initial_session,
        process_command,
        command_rx,
        event_tx,
        refresh_configuration,
    )
    .run();
}

impl Supervisor {
    fn new(
        project: PathBuf,
        draft_id: String,
        initial_session: Option<PathBuf>,
        process_command: AgentLaunchConfig,
        command_rx: mpsc::Receiver<RuntimeCommand>,
        event_tx: UiEventSender,
        refresh_configuration: bool,
    ) -> Self {
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
        let initial_command =
            initial_draft_command(draft_id, initial_project.clone(), initial_session.clone());
        let selected = initial_key.clone();
        let generation = 0_u64;
        let mut latest = HashMap::<String, Arc<RuntimeSnapshot>>::new();
        let catalog_sessions = Vec::<SessionSummary>::new();
        let catalog_generation = 0_u64;
        let activity_tracker = ExternalActivityTracker::default();
        if let Some(path) = initial_session.clone() {
            latest.insert(
                initial_key.clone(),
                Arc::new(RuntimeSnapshot {
                    project: initial_project.clone(),
                    selected_session: Some(path),
                    history_preview: true,
                    ..RuntimeSnapshot::default()
                }),
            );
        }
        let actor_paths = initial_session
            .map(|path| HashMap::from([(path, initial_key.clone())]))
            .unwrap_or_default();
        let interacted = HashSet::from([initial_key.clone()]);
        let document_revisions = HashMap::new();
        let pending_extensions = HashMap::<String, Vec<crate::protocol::ExtensionUiRequest>>::new();
        let active_dialogs = HashMap::<String, Vec<crate::protocol::ExtensionUiRequest>>::new();
        let needs_input = HashSet::<String>::new();
        let clock = 0_u64;
        let last_touch = HashMap::from([(initial_key.clone(), clock)]);
        let mut configurations = HarnessConfigurationStore::default();
        let catalog_state = StateStore::open().ok();
        let configuration_catalogs = catalog_state
            .as_ref()
            .and_then(|state| state.load_configuration_catalogs().ok())
            .unwrap_or_default();
        for entry in &configuration_catalogs {
            configurations.set_catalog(
                entry.harness.clone(),
                entry.project.clone(),
                entry.catalog.clone(),
            );
        }
        if let Some(state) = catalog_state.as_ref()
            && let Ok(defaults) = state.load_session_control_defaults()
        {
            configurations.restore(defaults);
        }
        if let Some(actor) = actors.get(&initial_key) {
            send_configured_command(actor, initial_command, &configurations);
        }
        let (configuration_tx, configuration_rx) = mpsc::channel();
        if refresh_configuration {
            refresh_configuration_catalogs(
                initial_project.clone(),
                process_command.clone(),
                supervisor_thread.clone(),
                configuration_tx,
            );
        }
        let published_statuses = HashMap::<String, (Option<PathBuf>, String)>::new();
        if let Ok(state) = StateStore::open()
            && let Ok(prompts) = agents::queued_prompts(&state)
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
        Self {
            process_command,
            command_rx,
            event_tx,
            supervisor_thread,
            catalog_key,
            actors,
            selected,
            generation,
            latest,
            catalog_sessions,
            catalog_generation,
            activity_tracker,
            actor_paths,
            interacted,
            document_revisions,
            pending_extensions,
            active_dialogs,
            needs_input,
            clock,
            last_touch,
            configurations,
            catalog_state,
            configuration_catalogs,
            configuration_rx,
            published_statuses,
        }
    }

    fn run(mut self) {
        let mut running = true;
        while running {
            self.drain_configuration_updates();
            self.maintain_external_activity();
            self.drain_actor_events();
            running = self.process_next_command();
        }
        for actor in self.actors.values() {
            actor.send(RuntimeCommand::Shutdown);
        }
        for actor in self.actors.into_values() {
            actor.join();
        }
        let _ = self.event_tx.send(RuntimeEvent::Stopped);
    }
}

pub(super) fn initial_draft_command(
    id: String,
    project: PathBuf,
    session: Option<PathBuf>,
) -> RuntimeCommand {
    session.map_or(
        RuntimeCommand::ResumeDraft {
            id,
            harness: "pi".into(),
            project: project.clone(),
        },
        |path| {
            let (harness, session_id) = agents::external_session_identity(&path)
                .unwrap_or_else(|| ("pi", path.to_string_lossy().into_owned()));
            RuntimeCommand::SelectSession {
                session_id,
                path,
                harness: harness.into(),
                project,
            }
        },
    )
}

#[derive(Debug)]
pub(super) enum SupervisorSessionAction {
    Publish(RuntimeEvent),
    RefreshCatalog,
}

pub(super) fn command_targets_catalog(command: &RuntimeCommand) -> bool {
    matches!(
        command,
        RuntimeCommand::LoadSessions(_)
            | RuntimeCommand::RefreshSessions
            | RuntimeCommand::ScheduleSessionRefresh
            | RuntimeCommand::SetSessionArchived { .. }
            | RuntimeCommand::RenameSession { .. }
            | RuntimeCommand::MoveSession { .. }
    )
}

pub(super) fn route_session_discovery(
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
        RuntimeCommand::NewSession { id, project, .. }
        | RuntimeCommand::ResumeDraft { id, project, .. } => {
            Some((format!("draft:{id}"), project.clone()))
        }
        RuntimeCommand::ForkSession { path, project, .. } => {
            Some((format!("fork:{}", path.display()), project.clone()))
        }
        RuntimeCommand::SelectSession { path, project, .. }
        | RuntimeCommand::RestartSession { path, project, .. } => {
            Some((format!("session:{}", path.display()), project.clone()))
        }
        _ => None,
    }
}

pub(super) fn is_view_only_selection(command: &RuntimeCommand) -> bool {
    matches!(command, RuntimeCommand::SelectSession { .. })
}

pub(super) fn target_command_needs_actor_message(
    view_only: bool,
    resident: Option<&RuntimeSnapshot>,
) -> bool {
    !view_only
        || resident.is_none()
        || resident.is_some_and(|snapshot| !snapshot.connected && !snapshot.history_preview)
}

pub(super) fn actor_key_for_command(
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
