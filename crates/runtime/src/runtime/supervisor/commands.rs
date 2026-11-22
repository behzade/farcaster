use super::*;
use crate::agents::Backend;

impl Supervisor {
    fn routed_actor_key(&self, command: &RuntimeCommand, requested: &str) -> String {
        if let RuntimeCommand::SelectSession {
            key,
            path,
            project,
            harness,
            session_id,
            ..
        }
        | RuntimeCommand::RestartSession {
            key,
            path,
            project,
            harness,
            session_id,
            ..
        } = command
        {
            if self.catalog_sessions.has_shared_path(path) {
                let matches = |row: &SessionSummary| {
                    row.path == *path
                        && row.harness == *harness
                        && row.project == *project
                        && row.id == *session_id
                };
                let row = match key {
                    Some(key) => self.catalog_sessions.by_key(key).filter(|row| matches(row)),
                    None => self.catalog_sessions.at_path(path).find(|row| matches(row)),
                };
                if let Some(row) = row {
                    return documents::catalog_actor_key(
                        row,
                        &self.catalog_sessions,
                        &self.actor_paths,
                        &self.latest,
                    );
                }
                if let Some(key) = key {
                    return documents::record_actor_key(key);
                }
            }
        }
        actor_key_for_command(command, requested, &self.actor_paths, &self.latest)
    }

    fn send_to_session(&mut self, command: RuntimeCommand) {
        let RuntimeCommand::SendToSession {
            submission_id,
            target,
            session,
            project,
            ..
        } = &command
        else {
            return;
        };
        let key = if let Some(session) = session {
            let config = match configuration_for_target(
                &self.process_command,
                self.catalog_state.as_ref(),
                LaunchTarget::Session(&session.path, Some(session.harness)),
            ) {
                Ok(config) => config,
                Err(error) => {
                    self.report_configuration_error(error);
                    let _ = self.event_tx.send(RuntimeEvent::PromptResult {
                        submission_id: Some(submission_id.clone()),
                        target: target.clone(),
                        outcome: crate::agents::PromptOutcome::RejectedBeforeAcceptance,
                        session: Some(session.path.clone()),
                    });
                    return;
                }
            };
            let access_mode = saved_access_mode(
                self.catalog_state.as_ref(),
                &session.path,
                Some(session.harness),
            );
            let select = RuntimeCommand::SelectSession {
                display_stamp: None,
                path: session.path.clone(),
                harness: session.harness,
                session_id: session.id.clone(),
                project: project.clone(),
                key: None,
                request: None,
            };
            let key = self.routed_actor_key(&select, target);
            let resident = self.latest.get(&key);
            let actor = self.actors.entry(key.clone()).or_insert_with(|| {
                SessionRuntimeHandle::spawn(
                    project.clone(),
                    self.process_command.clone(),
                    false,
                    Some(session.harness),
                    self.supervisor_thread.clone(),
                    self.host.clone(),
                )
            });
            if target_command_needs_actor_message(&select, resident.map(Arc::as_ref)) {
                send_configured_command(
                    actor,
                    select,
                    &self.configurations,
                    access_mode,
                    config.profile_id.as_deref(),
                );
            }
            self.actor_paths
                .insert(session.path.clone(), session.harness, key.clone());
            key
        } else {
            target.clone()
        };
        if let Some(actor) = self.actors.get(&key) {
            self.clock = self.clock.saturating_add(1);
            self.last_touch.insert(key.clone(), self.clock);
            self.interacted.insert(key);
            actor.send(command);
        } else {
            let _ = self.event_tx.send(RuntimeEvent::PromptResult {
                submission_id: Some(submission_id.clone()),
                target: target.clone(),
                outcome: crate::agents::PromptOutcome::RejectedBeforeAcceptance,
                session: session.as_ref().map(|session| session.path.clone()),
            });
        }
    }

    fn start_background_task(
        &mut self,
        submission_id: String,
        id: String,
        settings: TaskSettings,
        message: String,
    ) {
        let key = format!("draft:{id}");
        if self.actors.contains_key(&key) {
            return;
        }
        let mut process_command = self.process_command.clone();
        process_command.access_mode = settings.access_mode;
        let actor = SessionRuntimeHandle::spawn(
            settings.project.clone(),
            process_command,
            false,
            settings.harness,
            self.supervisor_thread.clone(),
            self.host.clone(),
        );
        if let Some(catalog) = self
            .configurations
            .catalog_command(settings.harness, &settings.project)
        {
            actor.send(catalog);
        }
        if let Some(model) = settings.model {
            actor.send(RuntimeCommand::SetModel(model));
        }
        if let Some(effort) = settings.effort {
            actor.send(RuntimeCommand::SetThinking(effort));
        }
        actor.send(RuntimeCommand::Prompt {
            submission_id,
            target: key.clone(),
            mode: PromptMode::Normal,
            provisional_title: crate::sessions::provisional_session_title(&message),
            message,
            display_message: None,
            invocation: None,
            images: Vec::new(),
            allow_while_running: false,
        });
        self.clock = self.clock.saturating_add(1);
        self.last_touch.insert(key.clone(), self.clock);
        self.interacted.insert(key.clone());
        self.latest.insert(
            key.clone(),
            Arc::new(RuntimeSnapshot {
                harness: settings.harness,
                project: settings.project,
                ..RuntimeSnapshot::default()
            }),
        );
        self.actors.insert(key, actor);
    }

    fn request_configuration(
        &mut self,
        harness: Backend,
        project: PathBuf,
        target: &str,
        launch_target: Option<LaunchTarget<'_>>,
    ) {
        let Some(sender) = &self.configuration_tx else {
            return;
        };
        let process_command =
            match self.configuration_process_command(harness, &project, target, launch_target) {
                Ok(config) => config,
                Err(error) => {
                    self.report_configuration_error(error);
                    return;
                }
            };
        let profile_id = process_command.profile_id.clone();
        if !self
            .configuration_requests
            .insert((harness, profile_id.clone(), project.clone()))
        {
            return;
        }
        self.configurations
            .set_catalog_loading(harness, profile_id.clone(), project.clone());
        let request_harness = harness;
        let request_project = project.clone();
        let request_profile_id = profile_id.clone();
        let supervisor = self.supervisor_thread.clone();
        let updates = sender.clone();
        if let Err(error) = thread::Builder::new()
            .name(format!("farcaster-{harness}-catalog"))
            .spawn(move || {
                let result = agents::refresh_configuration_catalog(
                    &process_command,
                    request_harness,
                    &request_project,
                );
                let _ =
                    updates.send((request_harness, request_profile_id, request_project, result));
                supervisor.unpark();
            })
        {
            let _ = sender.send((
                harness,
                profile_id.clone(),
                project.clone(),
                Err(format!("start catalog request: {error}")),
            ));
        }
        self.publish_configuration_snapshots(harness, profile_id.as_deref(), &project);
    }

    pub(super) fn configuration_process_command(
        &self,
        harness: Backend,
        project: &std::path::Path,
        target: &str,
        launch_target: Option<LaunchTarget<'_>>,
    ) -> Result<AgentLaunchConfig, String> {
        let launch_target = launch_target
            .or_else(|| {
                self.latest
                    .get(target)
                    .and_then(|snapshot| snapshot.selected_session.as_deref())
                    .or_else(|| {
                        self.actor_paths
                            .iter()
                            .find_map(|(path, key)| (key == target).then_some(path.as_path()))
                    })
                    .or_else(|| crate::sessions::session_path(target))
                    .map(|p| LaunchTarget::Session(p, Some(harness)))
                    .or_else(|| crate::sessions::draft_id(target).map(LaunchTarget::Draft))
            })
            .ok_or_else(|| format!("Missing launch target: {target}"))?;
        let mut command = configuration_for_target(
            &self.process_command,
            self.catalog_state.as_ref(),
            launch_target,
        )?;
        if let Some((requested_harness, requested_project, requested_mode)) =
            self.requested_access_modes.get(target)
            && *requested_harness == harness
            && requested_project.as_path() == project
        {
            command.access_mode = *requested_mode;
            return Ok(command);
        }
        if let Some(snapshot) = self.latest.get(target).filter(|snapshot| {
            snapshot.harness == Some(harness) && snapshot.project.as_path() == project
        }) {
            command.access_mode = snapshot.access_mode;
        }
        Ok(command)
    }

    fn report_configuration_error(&self, error: String) {
        let _ = self.event_tx.send(RuntimeEvent::SystemNotification {
            title: "Farcaster: Harness configuration unavailable".into(),
            body: error,
            target: None,
        });
    }

    fn signal_actor(&self, target: &str) -> Option<&SessionRuntimeHandle> {
        if let Some((harness, address)) = sessions::composer_buffer_parts(target) {
            let path = sessions::session_path(address)?;
            let matches = |snapshot: &Arc<RuntimeSnapshot>| {
                snapshot.harness == Some(harness)
                    && (snapshot.selected_session.as_deref() == Some(path)
                        || snapshot.live_session.as_deref() == Some(path))
            };
            if self.latest.get(&self.selected).is_some_and(matches) {
                return self.actors.get(&self.selected);
            }
            // A selection command can overtake a signal on the separate channel.
            let mut owners = self.latest.iter().filter(|(_, snapshot)| matches(snapshot));
            let (key, _) = owners.next()?;
            if owners.next().is_some() {
                return None;
            }
            return self.actors.get(key);
        }
        let key = sessions::session_path(target)
            .and_then(|path| self.actor_paths.last_at(path))
            .map_or(target, String::as_str);
        self.actors.get(key)
    }

    fn drain_signals(&self) {
        for (target, signal) in self.signal_rx.try_iter() {
            if let Some(actor) = self.signal_actor(&target) {
                actor.signal(signal);
            }
        }
    }

    pub(super) fn process_next_command(&mut self) -> bool {
        match super::super::command_queue::receive_command(
            &self.command_rx,
            &mut self.pending_command,
        ) {
            Ok(RuntimeCommand::Shutdown) => false,
            Ok(command) => {
                if matches!(command, RuntimeCommand::RefreshAccounts) {
                    self.accounts.refresh();
                    return true;
                }
                if matches!(command, RuntimeCommand::SystemWake) {
                    self.accounts.invalidate();
                    for actor in self.actors.values() {
                        actor.send(command.clone());
                    }
                    return true;
                }
                if matches!(command, RuntimeCommand::SendToSession { .. }) {
                    self.send_to_session(command);
                    return true;
                }
                if let RuntimeCommand::StartTask {
                    submission_id,
                    id,
                    settings,
                    message,
                } = command
                {
                    self.start_background_task(submission_id, id, settings, message);
                    return true;
                }
                if let RuntimeCommand::LoadConfiguration { harness, project } = &command {
                    let selected = self.selected.clone();
                    self.request_configuration(*harness, project.clone(), &selected, None);
                    return true;
                }
                if self.handle_session_family_command(&command) {
                    return true;
                }
                if let RuntimeCommand::ExtensionResponse(response) = &command {
                    let id = response.id();
                    if let Some(dialogs) = self.active_dialogs.get_mut(&self.selected) {
                        dialogs.retain(|request| request.dialog_id() != Some(id));
                        if !dialogs.iter().any(ExtensionUiRequest::is_blocking) {
                            self.needs_input.remove(&self.selected);
                        }
                        if dialogs.is_empty() {
                            self.active_dialogs.remove(&self.selected);
                            self.needs_input.remove(&self.selected);
                        }
                    }
                    publish_resident_status(
                        &self.event_tx,
                        &mut self.published_statuses,
                        &self.catalog_sessions,
                        &self.selected,
                        self.latest.get(&self.selected).map(AsRef::as_ref),
                        if self.needs_input.contains(&self.selected) {
                            RunStatus::NeedsInput
                        } else if agents::is_child_input_id(id)
                            || matches!(response, ExtensionUiResponse::Questions(_))
                        {
                            self.latest
                                .get(&self.selected)
                                .map_or(RunStatus::Done, |snapshot| semantic_status(snapshot))
                        } else {
                            RunStatus::Working
                        },
                    );
                }
                if let RuntimeCommand::SetAppProxy(proxy) = &command {
                    if let Err(error) = self.host.set_worker_app_proxy(proxy.clone()) {
                        let _ = self.event_tx.send(RuntimeEvent::SystemNotification {
                            title: "Farcaster: Worker proxy update failed".into(),
                            body: error,
                            target: None,
                        });
                    }
                    self.process_command.app_proxy = proxy.clone();
                    for actor in self.actors.values() {
                        actor.send(command.clone());
                    }
                    return true;
                }
                if let RuntimeCommand::SetAccessMode(mode) = &command
                    && let Some(snapshot) = self.latest.get(&self.selected)
                    && let Some(harness) = snapshot.harness
                {
                    self.requested_access_modes.insert(
                        self.selected.clone(),
                        (harness, snapshot.project.clone(), *mode),
                    );
                    if snapshot.live_session.is_none()
                        && let Some(session) = snapshot.selected_session.as_deref()
                        && let Some(state) = self.catalog_state.as_ref()
                        && let Err(error) = state.with(|store| {
                            store.set_session_access_mode_for_backend(session, Some(harness), *mode)
                        })
                    {
                        zlog::error!("Save session access mode: {error}");
                    }
                }
                let identity_changed = self.latest.get(&self.selected).is_some_and(|snapshot| {
                    adopts_selected_configuration(snapshot, &self.catalog_sessions)
                        && update_selected_configuration(
                            &mut self.configurations,
                            snapshot,
                            &command,
                        )
                });
                if identity_changed {
                    persist_configurations(self.catalog_state.as_ref(), &self.configurations);
                }
                if let RuntimeCommand::RenameSession { path, name, .. } = &command
                    && let Some((key, actor)) = self.actors.iter().find(|(key, _)| {
                        self.latest
                            .get(*key)
                            .and_then(|snapshot| snapshot.live_session.as_deref())
                            == Some(path.as_path())
                    })
                {
                    actor.send(RuntimeCommand::SetSessionName(name.clone()));
                    self.clock = self.clock.saturating_add(1);
                    self.last_touch.insert(key.clone(), self.clock);
                    return true;
                }
                let next = command_target(&command);
                if let Some((requested_key, project, harness)) = next {
                    let _selection_timing = is_view_only_selection(&command)
                        .then(|| self.host.timer(RuntimeMetric::RuntimeRoute));
                    let key = self.routed_actor_key(&command, &requested_key);
                    let sequence = match &command {
                        RuntimeCommand::NewSession { request, .. }
                        | RuntimeCommand::ResumeDraft { request, .. }
                        | RuntimeCommand::SelectSession { request, .. }
                        | RuntimeCommand::RestartSession { request, .. } => *request,
                        _ => None,
                    };
                    let profile_id = match configuration_for_target(
                        &self.process_command,
                        self.catalog_state.as_ref(),
                        LaunchTarget::for_command(&command).expect("routed session target"),
                    ) {
                        Ok(config) => config.profile_id,
                        Err(error) => {
                            if let Some(sequence) = sequence {
                                let _ = self
                                    .event_tx
                                    .send(RuntimeEvent::SelectionRejected(sequence));
                            }
                            self.report_configuration_error(error);
                            return true;
                        }
                    };
                    if matches!(
                        &command,
                        RuntimeCommand::NewSession { .. } | RuntimeCommand::ResumeDraft { .. }
                    ) && self.latest.get(&key).is_some_and(|snapshot| {
                        snapshot.harness != harness
                            || snapshot.project != project
                            || snapshot.profile_id != profile_id
                    }) {
                        // Only the actor can publish the draft under its new configuration.
                        self.latest.remove(&key);
                    }
                    if let Some(harness) = harness {
                        self.request_configuration(
                            harness,
                            project.clone(),
                            &key,
                            LaunchTarget::for_command(&command),
                        );
                    }
                    self.clock = self.clock.saturating_add(1);
                    self.last_touch.insert(key.clone(), self.clock);
                    self.interacted.insert(key.clone());
                    let selection_changed = key != self.selected;
                    let view_only = is_view_only_selection(&command);
                    if selection_changed {
                        self.generation = self.generation.saturating_add(1);
                        self.selected = key.clone();
                        if !view_only {
                            let _ = self.event_tx.send(RuntimeEvent::SessionReset {
                                generation: self.generation,
                                preserve_submission: false,
                            });
                        }
                    }
                    if let Some(sequence) = sequence {
                        let _ = self.event_tx.send(RuntimeEvent::SelectionRouted(sequence));
                    }
                    let next_selected_session = match &command {
                        RuntimeCommand::SelectSession { path, .. }
                        | RuntimeCommand::RestartSession { path, .. } => Some(path.clone()),
                        _ => None,
                    };
                    self.selected_project = project.clone();
                    self.selected_session = next_selected_session;
                    let resident_snapshot = self.latest.get(&key).filter(|snapshot| {
                        !matches!(&command, RuntimeCommand::SelectSession { display_stamp, .. } if snapshot.needs_saved_revision(*display_stamp))
                    }).cloned();
                    let access_mode = match &command {
                        RuntimeCommand::ForkSession { path, .. }
                        | RuntimeCommand::SelectSession { path, .. }
                        | RuntimeCommand::RestartSession { path, .. } => {
                            saved_access_mode(self.catalog_state.as_ref(), path, harness)
                        }
                        _ => None,
                    };
                    if let RuntimeCommand::SelectSession { path, harness, .. }
                    | RuntimeCommand::RestartSession { path, harness, .. } = &command
                    {
                        self.actor_paths.insert(path.clone(), *harness, key.clone());
                    }
                    let actor = self.actors.entry(key.clone()).or_insert_with(|| {
                        SessionRuntimeHandle::spawn(
                            project,
                            self.process_command.clone(),
                            false,
                            harness,
                            self.supervisor_thread.clone(),
                            self.host.clone(),
                        )
                    });
                    if target_command_needs_actor_message(&command, resident_snapshot.as_deref()) {
                        send_configured_command(
                            actor,
                            command,
                            &self.configurations,
                            access_mode,
                            profile_id.as_deref(),
                        );
                    }
                    if let Some(mut snapshot) = resident_snapshot {
                        if snapshot.profile_id != profile_id {
                            let snapshot = Arc::make_mut(&mut snapshot);
                            snapshot.profile_id = profile_id;
                        }
                        self.configurations
                            .refresh_snapshot_catalog(Arc::make_mut(&mut snapshot));
                        if let Some(mode) = access_mode {
                            Arc::make_mut(&mut snapshot).access_mode = mode;
                        }
                        if view_only {
                            Arc::make_mut(&mut snapshot).transcript_changed_from = None;
                        }
                        self.latest.insert(key.clone(), snapshot.clone());
                        let _ = self.event_tx.send(RuntimeEvent::Snapshot {
                            generation: self.generation,
                            snapshot,
                        });
                    }
                    if let Some(requests) = self.pending_extensions.remove(&key) {
                        for request in requests {
                            let _ = self.event_tx.send(RuntimeEvent::ExtensionUi {
                                generation: self.generation,
                                request,
                                system_notification_target: None,
                            });
                        }
                    }
                    if selection_changed && let Some(dialogs) = self.active_dialogs.get(&key) {
                        for request in dialogs {
                            let _ = self.event_tx.send(RuntimeEvent::ExtensionUi {
                                generation: self.generation,
                                request: request.clone(),
                                system_notification_target: None,
                            });
                        }
                    }
                } else {
                    let target = if command_targets_catalog(&command) {
                        &self.catalog_key
                    } else {
                        &self.selected
                    };
                    if let RuntimeCommand::SetSessionArchived { request, .. } = &command {
                        let request = request.clone();
                        if let Err(message) = self
                            .actors
                            .get(target)
                            .ok_or_else(|| "Session catalog is unavailable".to_owned())
                            .and_then(|actor| actor.try_send(command))
                        {
                            let _ = self.event_tx.send(RuntimeEvent::SessionArchiveResult {
                                request,
                                rows: Err(message),
                            });
                        }
                    } else if let Some(actor) = self.actors.get(target) {
                        actor.send(command);
                    }
                }
                true
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.drain_signals();
                thread::park_timeout(Duration::from_secs(30));
                true
            }
            Err(mpsc::TryRecvError::Disconnected) => false,
        }
    }
}
