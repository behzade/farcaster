use super::*;

impl Supervisor {
    pub(super) fn process_next_command(&mut self) -> bool {
        match self.command_rx.try_recv() {
            Ok(RuntimeCommand::Shutdown) => false,
            Ok(command) => {
                if self.handle_session_family_command(&command) {
                    return true;
                }
                if matches!(&command, RuntimeCommand::ExtensionResponse(_)) {
                    if let Some(dialogs) = self.active_dialogs.get_mut(&self.selected) {
                        if !dialogs.is_empty() {
                            dialogs.remove(0);
                        }
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
                        "Working",
                    );
                }
                if let RuntimeCommand::SetAppProxy(proxy) = &command {
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
                if let Some((requested_key, project)) = next {
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
                    let resident_snapshot = self.latest.get(&key).cloned();
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
                            self.supervisor_thread.clone(),
                        )
                    });
                    if target_command_needs_actor_message(view_only, resident_snapshot.as_deref()) {
                        send_configured_command(actor, command, &self.configurations);
                    }
                    if let Some(mut snapshot) = resident_snapshot {
                        if view_only {
                            Arc::make_mut(&mut snapshot).transcript_changed_from = None;
                        }
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
                    if let Some(actor) = self.actors.get(target) {
                        actor.send(command);
                    }
                }
                true
            }
            Err(mpsc::TryRecvError::Empty) => {
                match self.activity_tracker.next_deadline() {
                    Some(deadline) => {
                        thread::park_timeout(deadline.saturating_duration_since(Instant::now()))
                    }
                    None => thread::park(),
                }
                true
            }
            Err(mpsc::TryRecvError::Disconnected) => false,
        }
    }
}
