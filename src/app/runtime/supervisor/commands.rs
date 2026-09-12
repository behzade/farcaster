use super::*;

impl Supervisor {
    fn send_to_session(&mut self, command: RuntimeCommand) {
        let RuntimeCommand::SendToSession {
            target,
            session,
            project,
            ..
        } = &command
        else {
            return;
        };
        let key = if let Some(session) = session {
            let select = RuntimeCommand::SelectSession {
                path: session.path.clone(),
                harness: session.harness.clone(),
                session_id: session.id.clone(),
                project: project.clone(),
            };
            let key = self
                .actor_paths
                .get(&session.path)
                .cloned()
                .unwrap_or_else(|| actor_key_for_command(&select, target, &self.latest));
            let resident = self.latest.get(&key);
            let actor = self.actors.entry(key.clone()).or_insert_with(|| {
                SessionRuntimeHandle::spawn(
                    project.clone(),
                    self.process_command.clone(),
                    false,
                    session.harness.clone(),
                    self.supervisor_thread.clone(),
                )
            });
            if target_command_needs_actor_message(&select, resident.map(Arc::as_ref)) {
                send_configured_command(actor, select, &self.configurations);
            }
            self.actor_paths.insert(session.path.clone(), key.clone());
            key
        } else {
            // Draft comments address only the draft where the capture began.
            target.clone()
        };
        if let Some(actor) = self.actors.get(&key) {
            self.clock = self.clock.saturating_add(1);
            self.last_touch.insert(key.clone(), self.clock);
            self.interacted.insert(key);
            actor.send(command);
        } else {
            let _ = self.event_tx.send(RuntimeEvent::PromptResult {
                target: target.clone(),
                outcome: crate::agents::PromptOutcome::RejectedBeforeAcceptance,
                session: session.as_ref().map(|session| session.path.clone()),
            });
        }
    }

    fn start_background_task(&mut self, id: String, settings: TaskSettings, message: String) {
        let key = format!("draft:{id}");
        // Retrying the same creation request must not launch or submit twice.
        if self.actors.contains_key(&key) {
            return;
        }
        let mut process_command = self.process_command.clone();
        process_command.access_mode = settings.access_mode;
        let actor = SessionRuntimeHandle::spawn(
            settings.project.clone(),
            process_command,
            false,
            settings.harness.clone(),
            self.supervisor_thread.clone(),
        );
        if let Some(catalog) = self
            .configurations
            .catalog_command(&settings.harness, &settings.project)
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
            target: key.clone(),
            mode: PromptMode::Normal,
            message,
            display_message: None,
            invocation: None,
            images: Vec::new(),
            allow_while_running: false,
        });
        self.clock = self.clock.saturating_add(1);
        self.last_touch.insert(key.clone(), self.clock);
        self.interacted.insert(key.clone());
        // A user may open the draft before its actor has published any events.
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

    fn request_configuration(&mut self, harness: String, project: PathBuf) {
        let Some(sender) = &self.configuration_tx else {
            return;
        };
        if harness.is_empty()
            || !self
                .configuration_requests
                .insert((harness.clone(), project.clone()))
        {
            return;
        }
        self.configurations
            .set_catalog_loading(harness.clone(), project.clone());
        let request_harness = harness.clone();
        let request_project = project.clone();
        let process_command = self.process_command.clone();
        let supervisor = self.supervisor_thread.clone();
        let updates = sender.clone();
        if let Err(error) = thread::Builder::new()
            .name(format!("farcaster-{harness}-catalog"))
            .spawn(move || {
                let result = agents::load_configuration_catalog(
                    &process_command,
                    &request_harness,
                    &request_project,
                );
                let _ = updates.send((request_harness, request_project, result));
                supervisor.unpark();
            })
        {
            let _ = sender.send((
                harness.clone(),
                project.clone(),
                Err(format!("start catalog request: {error}")),
            ));
        }
        self.publish_configuration_snapshots(&harness, &project);
    }

    pub(super) fn process_next_command(&mut self) -> bool {
        match self.command_rx.try_recv() {
            Ok(RuntimeCommand::Shutdown) => false,
            Ok(command) => {
                if matches!(command, RuntimeCommand::SendToSession { .. }) {
                    self.send_to_session(command);
                    return true;
                }
                if let RuntimeCommand::StartTask {
                    id,
                    settings,
                    message,
                } = command
                {
                    self.start_background_task(id, settings, message);
                    return true;
                }
                if let RuntimeCommand::LoadConfiguration { harness, project } = &command {
                    self.request_configuration(harness.clone(), project.clone());
                    return true;
                }
                if self.handle_session_family_command(&command) {
                    return true;
                }
                if let RuntimeCommand::ExtensionResponse(response) = &command {
                    if self.resolve_recovery_response(response) {
                        return true;
                    }
                    let id = match response {
                        ExtensionUiResponse::Value { id, .. }
                        | ExtensionUiResponse::Confirmed { id, .. }
                        | ExtensionUiResponse::Cancelled { id, .. } => id,
                    };
                    if let Some(dialogs) = self.active_dialogs.get_mut(&self.selected) {
                        dialogs.retain(|request| request.dialog_id() != Some(id.as_str()));
                        if dialogs.is_empty() {
                            self.active_dialogs.remove(&self.selected);
                            self.needs_input.remove(&self.selected);
                        }
                    }
                    let session = self.latest.get(&self.selected).and_then(|snapshot| {
                        snapshot
                            .live_session
                            .clone()
                            .or_else(|| snapshot.selected_session.clone())
                    });
                    publish_session_status_if_changed(
                        &self.event_tx,
                        &mut self.published_statuses,
                        &self.selected,
                        session,
                        if self.needs_input.contains(&self.selected) {
                            "Needs input"
                        } else if agents::is_child_input_id(id) {
                            self.latest
                                .get(&self.selected)
                                .map_or("Done", |snapshot| semantic_status(snapshot))
                        } else {
                            "Working"
                        },
                    );
                }
                if let RuntimeCommand::SetAppProxy(proxy) = &command {
                    if let Err(error) = crate::app::mcp_server::set_worker_app_proxy(proxy.clone())
                    {
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
                    self.request_configuration(harness.clone(), project.clone());
                    let _selection_timing = is_view_only_selection(&command).then(|| {
                        crate::app::infrastructure::performance::Timing::new("switch.runtime_route")
                    });
                    let key = match &command {
                        RuntimeCommand::SelectSession { path, .. }
                        | RuntimeCommand::RestartSession { path, .. } => {
                            self.actor_paths.get(path).cloned().unwrap_or_else(|| {
                                actor_key_for_command(&command, &requested_key, &self.latest)
                            })
                        }
                        _ => requested_key,
                    };
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
                    let next_selected_session = match &command {
                        RuntimeCommand::SelectSession { path, .. }
                        | RuntimeCommand::RestartSession { path, .. } => Some(path.clone()),
                        _ => None,
                    };
                    self.selected_project = project.clone();
                    self.selected_session = next_selected_session;
                    let resident_snapshot = self.latest.get(&key).cloned();
                    let recovery_can_publish = resident_snapshot.is_some();
                    if let RuntimeCommand::SelectSession { path, .. }
                    | RuntimeCommand::RestartSession { path, .. } = &command
                    {
                        self.actor_paths.insert(path.clone(), key.clone());
                    }
                    let actor = self.actors.entry(key.clone()).or_insert_with(|| {
                        SessionRuntimeHandle::spawn(
                            project,
                            self.process_command.clone(),
                            false,
                            harness,
                            self.supervisor_thread.clone(),
                        )
                    });
                    if target_command_needs_actor_message(&command, resident_snapshot.as_deref()) {
                        send_configured_command(actor, command, &self.configurations);
                    }
                    if let Some(mut snapshot) = resident_snapshot {
                        self.configurations
                            .refresh_snapshot_catalog(Arc::make_mut(&mut snapshot));
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
                    if recovery_can_publish {
                        self.publish_selected_recovery_dialogs();
                    }
                } else {
                    let target = if command_targets_catalog(&command) {
                        &self.catalog_key
                    } else {
                        &self.selected
                    };
                    if let Some(actor) = self.actors.get(target) {
                        actor.send(command);
                    }
                }
                true
            }
            Err(mpsc::TryRecvError::Empty) => {
                thread::park();
                true
            }
            Err(mpsc::TryRecvError::Disconnected) => false,
        }
    }
}
