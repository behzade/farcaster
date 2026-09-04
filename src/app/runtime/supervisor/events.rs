use super::*;

impl Supervisor {
    pub(super) fn drain_configuration_updates(&mut self) {
        while let Ok((harness, project, result)) = self.configuration_rx.try_recv() {
            match result {
                Ok(catalog) => {
                    self.configurations.set_catalog(
                        harness.clone(),
                        project.clone(),
                        catalog.clone(),
                    );
                    if cache_configuration_catalog(
                        &mut self.configuration_catalogs,
                        harness.clone(),
                        project.clone(),
                        catalog.clone(),
                    ) && let Some(state) = self.catalog_state.as_ref()
                    {
                        let _ = state.save_configuration_catalogs(&self.configuration_catalogs);
                    }
                    if let Some(snapshot) = self.latest.get(&self.selected)
                        && snapshot.harness == harness
                        && snapshot.project == project
                    {
                        let mut updated = snapshot.clone();
                        let snapshot = Arc::make_mut(&mut updated);
                        snapshot.models.clone_from(&catalog.models);
                        snapshot.thinking_levels.clone_from(&catalog.efforts);
                        snapshot.configuration_status = ConfigurationStatus::Loaded;
                        self.latest.insert(self.selected.clone(), updated.clone());
                        let _ = self.event_tx.send(RuntimeEvent::Snapshot {
                            generation: self.generation,
                            snapshot: updated,
                        });
                    }
                }
                Err(error) => {
                    zlog::warn!("Failed to refresh {harness} catalog: {error}");
                    self.configurations.set_catalog_error(
                        harness.clone(),
                        project.clone(),
                        error.clone(),
                    );
                    if let Some(snapshot) = self.latest.get(&self.selected)
                        && snapshot.harness == harness
                        && snapshot.project == project
                    {
                        let mut updated = snapshot.clone();
                        let snapshot = Arc::make_mut(&mut updated);
                        snapshot.configuration_status = ConfigurationStatus::Failed(error);
                        self.latest.insert(self.selected.clone(), updated.clone());
                        let _ = self.event_tx.send(RuntimeEvent::Snapshot {
                            generation: self.generation,
                            snapshot: updated,
                        });
                    }
                }
            }
        }
    }

    pub(super) fn maintain_external_activity(&mut self) {
        let owned_sessions = rpc_owned_session_paths(&self.latest);
        self.activity_tracker.remove_owned(&owned_sessions);
        if self.activity_tracker.take_expired(Instant::now())
            && let Some(catalog) = self.actors.get(&self.catalog_key)
        {
            catalog.send(RuntimeCommand::RefreshSessions);
        }
    }

    pub(super) fn drain_actor_events(&mut self) {
        let keys = self.actors.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            let mut events = Vec::new();
            if let Some(actor) = self.actors.get(&key) {
                while let Ok(event) = actor.events.try_recv() {
                    events.push(event);
                }
            }
            for event in events {
                self.clock = self.clock.saturating_add(1);
                self.last_touch.insert(key.clone(), self.clock);
                self.handle_actor_event(key.clone(), event);
            }
        }
    }

    fn handle_actor_event(&mut self, key: String, event: RuntimeEvent) {
        match event {
            RuntimeEvent::Snapshot { snapshot, .. } => {
                let mut snapshot = snapshot;
                if !snapshot.models.is_empty()
                    && cache_configuration_catalog(
                        &mut self.configuration_catalogs,
                        snapshot.harness.clone(),
                        snapshot.project.clone(),
                        crate::agents::ConfigurationCatalog {
                            models: snapshot.models.clone(),
                            efforts: snapshot.thinking_levels.clone(),
                        },
                    )
                    && let Some(state) = self.catalog_state.as_ref()
                {
                    let _ = state.save_configuration_catalogs(&self.configuration_catalogs);
                }
                let adopts_identity = key == self.selected
                    && adopts_selected_configuration(&snapshot, &self.catalog_sessions);
                let identity_changed = self
                    .configurations
                    .reconcile_snapshot(Arc::make_mut(&mut snapshot), adopts_identity);
                if identity_changed {
                    persist_configurations(self.catalog_state.as_ref(), &self.configurations);
                }
                if snapshot.conversation.settled {
                    self.needs_input.remove(&key);
                    self.active_dialogs.remove(&key);
                }
                let status = if self.needs_input.contains(&key) {
                    "Needs input"
                } else {
                    semantic_status(&snapshot)
                };
                publish_session_status_if_changed(
                    &self.event_tx,
                    &mut self.published_statuses,
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
                    self.actor_paths.insert(path, key.clone());
                }
                self.latest.insert(key.clone(), snapshot.clone());
                if key == self.selected {
                    let _ = self.event_tx.send(RuntimeEvent::Snapshot {
                        generation: self.generation,
                        snapshot,
                    });
                }
            }
            RuntimeEvent::ExtensionUi { request, .. } => {
                if request.gpui_system_notification().is_some() {
                    let system_notification_target = self
                        .latest
                        .get(&key)
                        .and_then(|snapshot| notification_target(snapshot));
                    let _ = self.event_tx.send(RuntimeEvent::ExtensionUi {
                        generation: self.generation,
                        request,
                        system_notification_target,
                    });
                    return;
                }
                if request.dialog_id().is_some() {
                    self.active_dialogs
                        .entry(key.clone())
                        .or_default()
                        .push(request.clone());
                    self.needs_input.insert(key.clone());
                    let session = self.latest.get(&key).and_then(|snapshot| {
                        snapshot
                            .live_session
                            .clone()
                            .or_else(|| snapshot.selected_session.clone())
                    });
                    publish_session_status_if_changed(
                        &self.event_tx,
                        &mut self.published_statuses,
                        &key,
                        session,
                        "Needs input",
                    );
                }
                if key == self.selected {
                    let _ = self.event_tx.send(RuntimeEvent::ExtensionUi {
                        generation: self.generation,
                        request,
                        system_notification_target: None,
                    });
                } else if request.dialog_id().is_none() {
                    self.pending_extensions
                        .entry(key.clone())
                        .or_default()
                        .push(request);
                }
            }
            RuntimeEvent::SessionReset {
                preserve_submission,
                ..
            } if key == self.selected => {
                let _ = self.event_tx.send(RuntimeEvent::SessionReset {
                    generation: self.generation,
                    preserve_submission,
                });
            }
            RuntimeEvent::HistoryReset { .. } if key == self.selected => {
                let _ = self.event_tx.send(RuntimeEvent::HistoryReset {
                    generation: self.generation,
                });
            }
            RuntimeEvent::PromptResult {
                target,
                accepted,
                session,
                ..
            } => {
                let _ = self.event_tx.send(RuntimeEvent::PromptResult {
                    generation: self.generation,
                    target,
                    accepted,
                    session,
                });
            }
            RuntimeEvent::RefreshCatalog => {
                if let Some(catalog) = self.actors.get(&self.catalog_key) {
                    catalog.send(RuntimeCommand::RefreshSessions);
                }
            }
            RuntimeEvent::SessionFilesModified { paths } if key == self.catalog_key => {
                for (actor_key, path, project) in changed_external_documents(&self.latest, &paths) {
                    if let Some(actor) = self.actors.get(&actor_key) {
                        actor.send(RuntimeCommand::RefreshSessionDocument { path, project });
                    }
                }
                let refresh = self.activity_tracker.observe_files(
                    &rpc_owned_session_paths(&self.latest),
                    &paths,
                    Instant::now(),
                    sessions::normalize_session_path,
                );
                if refresh && let Some(catalog) = self.actors.get(&self.catalog_key) {
                    catalog.send(RuntimeCommand::ScheduleSessionRefresh);
                }
            }
            event @ (RuntimeEvent::Sessions { .. } | RuntimeEvent::SessionsFailed { .. }) => {
                if key == self.catalog_key
                    && let RuntimeEvent::Sessions {
                        generation: next_generation,
                        all_sessions,
                        activities,
                        ..
                    } = &event
                {
                    self.catalog_generation = *next_generation;
                    if let Some((_, exhaustive)) = activities {
                        self.activity_tracker.sync_catalog(
                            all_sessions,
                            *exhaustive,
                            &rpc_owned_session_paths(&self.latest),
                            Instant::now(),
                            SystemTime::now(),
                        );
                    }
                    self.catalog_sessions.clone_from(all_sessions);
                    reconcile_live_session_documents(
                        all_sessions,
                        &self.interacted,
                        &self.selected,
                        &mut self.actors,
                        &mut self.latest,
                        &mut self.last_touch,
                        &mut self.document_revisions,
                        &mut self.actor_paths,
                        &self.process_command,
                        &self.supervisor_thread,
                    );
                }
                match route_session_discovery(&key, &self.catalog_key, event) {
                    SupervisorSessionAction::Publish(event) => {
                        let _ = self.event_tx.send(event);
                    }
                    SupervisorSessionAction::RefreshCatalog => {
                        if let Some(catalog) = self.actors.get(&self.catalog_key) {
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
