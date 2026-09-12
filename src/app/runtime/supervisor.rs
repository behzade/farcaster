use super::*;

mod commands;
mod events;
mod family_commands;
#[cfg(test)]
#[path = "supervisor_proxy_tests.rs"]
mod proxy_tests;
mod recovery;

pub(crate) struct RuntimeHandle {
    pub(crate) session_targets: HashMap<PathBuf, crate::sessions::SessionTarget>,
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
        draft: crate::projects::DraftSession,
        initial_session: Option<crate::sessions::SessionTarget>,
        app_proxy: Option<String>,
    ) -> Self {
        let command = AgentLaunchConfig {
            app_proxy,
            session_locator_root: crate::app::paths::data_dir()
                .ok()
                .map(|root| root.join("session-locators")),
            ..AgentLaunchConfig::default()
        };
        Self::spawn_with_configuration_refresh(project, draft, initial_session, command, true)
    }

    #[cfg(test)]
    pub(crate) fn spawn_with(
        project: PathBuf,
        draft_id: String,
        initial_session: Option<crate::sessions::SessionTarget>,
        process_command: AgentLaunchConfig,
    ) -> Self {
        Self::spawn_with_configuration_refresh(
            project.clone(),
            crate::projects::DraftSession::with_id("pi".into(), draft_id, project),
            initial_session,
            process_command,
            false,
        )
    }

    fn spawn_with_configuration_refresh(
        project: PathBuf,
        draft: crate::projects::DraftSession,
        initial_session: Option<crate::sessions::SessionTarget>,
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
                    draft,
                    initial_session,
                    process_command,
                    command_rx,
                    event_tx,
                    refresh_configuration,
                );
            })
            .expect("start session supervisor");
        Self {
            session_targets: HashMap::new(),
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
            .map_err(|_| "Session runtime has stopped".to_owned())?;
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
    join: thread::JoinHandle<Result<(), String>>,
}

impl SessionRuntimeHandle {
    pub(super) fn spawn(
        project: PathBuf,
        process_command: AgentLaunchConfig,
        load_catalog: bool,
        harness: String,
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
            .spawn(move || {
                run(
                    project,
                    process_command,
                    command_rx,
                    event_tx,
                    load_catalog,
                    harness,
                )
            })
            .expect("start session runtime");
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

    fn join(self) -> Result<(), String> {
        self.join
            .join()
            .map_err(|_| "session runtime thread panicked during shutdown".to_owned())?
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

#[cfg(test)]
pub(super) fn changed_external_documents(
    latest: &HashMap<String, Arc<RuntimeSnapshot>>,
    paths: &[PathBuf],
) -> Vec<(String, PathBuf, PathBuf, String)> {
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
                Some((
                    key.clone(),
                    path.clone(),
                    snapshot.project.clone(),
                    snapshot.harness.clone(),
                ))
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
    let catalog = command_target(&command)
        .and_then(|(_, project, harness)| configurations.catalog_command(&harness, &project));
    // Fork and restart launch inside the command handler, so validate them
    // against the cached catalog before starting the child process.
    if matches!(
        &command,
        RuntimeCommand::ForkSession { .. } | RuntimeCommand::RestartSession { .. }
    ) && let Some(catalog) = &catalog
    {
        actor.send(catalog.clone());
    }
    actor.send(command);
    // The actor may have changed projects and rejected the earlier update.
    if let Some(catalog) = catalog {
        actor.send(catalog);
    }
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

type ConfigurationUpdate = (
    String,
    PathBuf,
    Result<crate::agents::ConfigurationCatalog, String>,
);

struct Supervisor {
    process_command: AgentLaunchConfig,
    command_rx: mpsc::Receiver<RuntimeCommand>,
    event_tx: UiEventSender,
    supervisor_thread: thread::Thread,
    catalog_key: String,
    actors: HashMap<String, SessionRuntimeHandle>,
    selected: String,
    selected_project: PathBuf,
    selected_session: Option<PathBuf>,
    generation: u64,
    latest: HashMap<String, Arc<RuntimeSnapshot>>,
    catalog_sessions: Vec<SessionSummary>,
    catalog_generation: u64,
    actor_paths: HashMap<PathBuf, String>,
    failed_actor_shutdowns: HashMap<PathBuf, String>,
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
    configuration_rx: mpsc::Receiver<ConfigurationUpdate>,
    configuration_tx: Option<mpsc::Sender<ConfigurationUpdate>>,
    // Coalesce in-flight requests and keep successful loads for this app run.
    // A failed result removes its key so the next selection can retry.
    configuration_requests: HashSet<(String, PathBuf)>,
    published_statuses: HashMap<String, (Option<PathBuf>, String)>,
    recovery: crate::app::runtime::recovery::InterruptedPromptRecovery,
    published_recovery_selection: Option<(u64, String, PathBuf, Option<PathBuf>)>,
}

fn run_supervisor(
    project: PathBuf,
    draft: crate::projects::DraftSession,
    initial_session: Option<crate::sessions::SessionTarget>,
    process_command: AgentLaunchConfig,
    command_rx: mpsc::Receiver<RuntimeCommand>,
    event_tx: UiEventSender,
    refresh_configuration: bool,
) {
    Supervisor::new(
        project,
        draft,
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
        draft: crate::projects::DraftSession,
        initial_session: Option<crate::sessions::SessionTarget>,
        process_command: AgentLaunchConfig,
        command_rx: mpsc::Receiver<RuntimeCommand>,
        event_tx: UiEventSender,
        refresh_configuration: bool,
    ) -> Self {
        let supervisor_thread = thread::current();
        let initial_key = format!("draft:{}", draft.id);
        let catalog_key = "catalog".to_owned();
        let initial_project = project.clone();
        let initial_command = initial_draft_command(draft, initial_session.clone());
        let initial_harness = session_actor_harness(&initial_command);
        let mut actors = HashMap::from([
            (
                catalog_key.clone(),
                SessionRuntimeHandle::spawn(
                    project.clone(),
                    process_command.clone(),
                    true,
                    String::new(),
                    supervisor_thread.clone(),
                ),
            ),
            (
                initial_key.clone(),
                SessionRuntimeHandle::spawn(
                    project,
                    process_command.clone(),
                    false,
                    initial_harness,
                    supervisor_thread.clone(),
                ),
            ),
        ]);
        let selected = initial_key.clone();
        let generation = 0_u64;
        let mut latest = HashMap::<String, Arc<RuntimeSnapshot>>::new();
        let catalog_sessions = Vec::<SessionSummary>::new();
        let catalog_generation = 0_u64;
        if let Some(target) = initial_session.clone() {
            latest.insert(
                initial_key.clone(),
                Arc::new(RuntimeSnapshot {
                    project: initial_project.clone(),
                    selected_session: Some(target.path),
                    harness: target.harness,
                    history_preview: true,
                    ..RuntimeSnapshot::default()
                }),
            );
        }
        let actor_paths = initial_session
            .as_ref()
            .map(|target| HashMap::from([(target.path.clone(), initial_key.clone())]))
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
        let recovery = catalog_state
            .as_ref()
            .map(crate::app::runtime::recovery::InterruptedPromptRecovery::recover)
            .transpose();
        let recovery = match recovery {
            Ok(Some(recovery)) => recovery,
            Ok(None) => Default::default(),
            Err(error) => {
                let _ = event_tx.send(RuntimeEvent::SystemNotification {
                    title: "Farcaster: Prompt recovery failed".into(),
                    body: error,
                    target: None,
                });
                Default::default()
            }
        };
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
                        prompt.harness.clone(),
                        supervisor_thread.clone(),
                    )
                });
                actor.send(RuntimeCommand::DeliverQueued(prompt));
            }
        }
        let selected_session = initial_session.as_ref().map(|target| target.path.clone());
        let mut supervisor = Self {
            process_command,
            command_rx,
            event_tx,
            supervisor_thread,
            catalog_key,
            actors,
            selected,
            selected_project: initial_project,
            selected_session,
            generation,
            latest,
            catalog_sessions,
            catalog_generation,
            actor_paths,
            failed_actor_shutdowns: HashMap::new(),
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
            configuration_tx: refresh_configuration.then_some(configuration_tx),
            configuration_requests: HashSet::new(),
            published_statuses,
            recovery,
            published_recovery_selection: None,
        };
        supervisor.publish_recovery_statuses();
        supervisor
    }

    fn run(mut self) {
        let mut running = true;
        while running {
            self.drain_configuration_updates();
            self.drain_actor_events();
            running = self.process_next_command();
        }
        for actor in self.actors.values() {
            actor.send(RuntimeCommand::Shutdown);
        }
        for actor in self.actors.into_values() {
            let _ = actor.join();
        }
        let _ = self.event_tx.send(RuntimeEvent::Stopped);
    }
}

pub(super) fn initial_draft_command(
    draft: crate::projects::DraftSession,
    session: Option<crate::sessions::SessionTarget>,
) -> RuntimeCommand {
    let project = draft.project;
    session.map_or(
        RuntimeCommand::ResumeDraft {
            id: draft.id,
            harness: draft.harness,
            project: project.clone(),
        },
        |target| RuntimeCommand::SelectSession {
            session_id: target.id,
            path: target.path,
            harness: target.harness,
            project,
        },
    )
}

#[derive(Debug)]
pub(super) enum SupervisorSessionAction {
    Publish(Box<RuntimeEvent>),
    RefreshCatalog,
}

pub(super) fn command_targets_catalog(command: &RuntimeCommand) -> bool {
    matches!(
        command,
        RuntimeCommand::LoadSessions(_)
            | RuntimeCommand::RefreshSessions
            | RuntimeCommand::UpdateSessionMetadata(_)
            | RuntimeCommand::ScheduleSessionRefresh
            | RuntimeCommand::SetSessionArchived { .. }
            | RuntimeCommand::RenameSession { .. }
            | RuntimeCommand::MoveSession { .. }
            | RuntimeCommand::PreviewImport { .. }
            | RuntimeCommand::CommitImport { .. }
    )
}

pub(super) fn route_session_discovery(
    actor_key: &str,
    catalog_key: &str,
    event: RuntimeEvent,
) -> SupervisorSessionAction {
    if actor_key == catalog_key {
        SupervisorSessionAction::Publish(Box::new(event))
    } else {
        SupervisorSessionAction::RefreshCatalog
    }
}

fn session_actor_harness(command: &RuntimeCommand) -> String {
    command_target(command)
        .map(|(_, _, harness)| harness)
        .expect("session actors spawn from a harnessed command")
}

fn command_target(command: &RuntimeCommand) -> Option<(String, PathBuf, String)> {
    match command {
        RuntimeCommand::NewSession {
            id,
            project,
            harness,
            ..
        }
        | RuntimeCommand::ResumeDraft {
            id,
            project,
            harness,
            ..
        } => Some((format!("draft:{id}"), project.clone(), harness.clone())),
        RuntimeCommand::ForkSession {
            path,
            project,
            harness,
            ..
        } => Some((
            format!("fork:{}", path.display()),
            project.clone(),
            harness.clone(),
        )),
        RuntimeCommand::SelectSession {
            path,
            project,
            harness,
            ..
        }
        | RuntimeCommand::RestartSession {
            path,
            project,
            harness,
            ..
        } => Some((
            format!("session:{}", path.display()),
            project.clone(),
            harness.clone(),
        )),
        _ => None,
    }
}

pub(super) fn is_view_only_selection(command: &RuntimeCommand) -> bool {
    matches!(command, RuntimeCommand::SelectSession { .. })
}

pub(super) fn target_command_needs_actor_message(
    command: &RuntimeCommand,
    resident: Option<&RuntimeSnapshot>,
) -> bool {
    let Some(snapshot) = resident else {
        return true;
    };
    match command {
        RuntimeCommand::ResumeDraft {
            harness, project, ..
        } => snapshot.harness != *harness || snapshot.project != *project,
        RuntimeCommand::SelectSession { harness, .. } => {
            snapshot.harness != *harness || (!snapshot.connected && !snapshot.history_preview)
        }
        _ => true,
    }
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

#[cfg(test)]
#[path = "supervisor_tests.rs"]
mod harness_birth_tests;

#[cfg(test)]
#[path = "catalog_lifecycle_tests.rs"]
mod catalog_lifecycle_tests;
