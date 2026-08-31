//! Runtime thread handles and multi-session command routing.

use super::*;

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
    pub(crate) fn spawn_with_grants(
        project: PathBuf,
        draft_id: String,
        initial_session: Option<PathBuf>,
        grants: crate::access::GrantStore,
        app_proxy: Option<String>,
    ) -> Self {
        let command = AgentLaunchConfig {
            grants: Some(grants),
            app_proxy,
            session_locator_root: crate::app::paths::data_dir()
                .ok()
                .map(|root| root.join("session-locators")),
            ..AgentLaunchConfig::default()
        };
        Self::spawn_with(project, draft_id, initial_session, command)
    }

    pub(crate) fn spawn_with(
        project: PathBuf,
        draft_id: String,
        initial_session: Option<PathBuf>,
        process_command: AgentLaunchConfig,
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

fn run_supervisor(
    project: PathBuf,
    draft_id: String,
    initial_session: Option<PathBuf>,
    mut process_command: AgentLaunchConfig,
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
                        for (actor_key, path, project) in
                            changed_external_documents(&latest, &paths)
                        {
                            if let Some(actor) = actors.get(&actor_key) {
                                actor
                                    .send(RuntimeCommand::RefreshSessionDocument { path, project });
                            }
                        }
                        let refresh = activity_tracker.observe_files(
                            &catalog_sessions,
                            &rpc_owned_session_paths(&latest),
                            &paths,
                            Instant::now(),
                            sessions::normalize_session_path,
                        );
                        if refresh && let Some(catalog) = actors.get(&catalog_key) {
                            catalog.send(RuntimeCommand::RefreshSessions);
                        }
                    }
                    event @ (RuntimeEvent::Sessions { .. }
                    | RuntimeEvent::SessionsFailed { .. }) => {
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
                        let family = archived_root_family_for_path(&catalog_sessions, path)
                            .ok_or_else(|| {
                                "Only an archived root session can be deleted".to_owned()
                            })?;
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
                        if agents::has_queued_prompts_for(&state, &paths)? {
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
                        let leftovers = delete_session_files(&paths)?;
                        let state_warning = sessions::delete_state(&mut state, &paths).err();
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
                            if let Some(catalog) = actors.get(&catalog_key) {
                                catalog.send(RuntimeCommand::RefreshSessions);
                            }
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
                        if agents::has_queued_prompts_for(&state, &paths)? {
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
                        let destination = sessions::destination_directory(
                            &session_root,
                            target_project,
                            &root.path,
                        );
                        let moved = sessions::move_family(
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
                        let state_warning =
                            sessions::relocate_state(&mut state, &path_updates, target_project)
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
                if let RuntimeCommand::SetAppProxy(proxy) = &command {
                    process_command.app_proxy = proxy.clone();
                    for actor in actors.values() {
                        actor.send(command.clone());
                    }
                    continue;
                }
                if matches!(command, RuntimeCommand::ReloadSandboxGrants) {
                    for actor in actors.values() {
                        actor.send(command.clone());
                    }
                    continue;
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
                    let _selection_timing = is_view_only_selection(&command).then(|| {
                        crate::app::infrastructure::performance::Timing::new("switch.runtime_route")
                    });
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
                            | RuntimeCommand::SetSessionArchived { .. }
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
        |path| RuntimeCommand::SelectSession {
            session_id: path.to_string_lossy().into_owned(),
            path,
            harness: "pi".into(),
            project,
        },
    )
}

#[derive(Debug)]
pub(super) enum SupervisorSessionAction {
    Publish(RuntimeEvent),
    RefreshCatalog,
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
