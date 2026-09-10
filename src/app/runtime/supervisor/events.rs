use super::*;
use std::path::Path;

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
                        catalog,
                    ) && let Some(state) = self.catalog_state.as_ref()
                    {
                        let _ = state.save_configuration_catalogs(&self.configuration_catalogs);
                    }
                }
                Err(error) => {
                    self.configuration_requests
                        .remove(&(harness.clone(), project.clone()));
                    zlog::warn!("Failed to refresh {harness} catalog: {error}");
                    self.configurations
                        .set_catalog_error(harness.clone(), project.clone(), error);
                }
            }
            self.publish_configuration_snapshots(&harness, &project);
        }
    }

    pub(super) fn publish_configuration_snapshots(&mut self, harness: &str, project: &Path) {
        for (key, snapshot) in &mut self.latest {
            if snapshot.harness == harness && snapshot.project == project {
                if let Some(actor) = self.actors.get(key)
                    && let Some(command) = self.configurations.catalog_command(harness, project)
                {
                    actor.send(command);
                }
                self.configurations
                    .refresh_snapshot_catalog(Arc::make_mut(snapshot));
                if key == &self.selected {
                    let _ = self.event_tx.send(RuntimeEvent::Snapshot {
                        generation: self.generation,
                        snapshot: snapshot.clone(),
                    });
                }
            }
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
            RuntimeEvent::SessionMetadata(metadata) => {
                if let Some(catalog) = self.actors.get(&self.catalog_key) {
                    catalog.send(RuntimeCommand::UpdateSessionMetadata(metadata));
                }
            }
            RuntimeEvent::SessionUpdated(session) => {
                if let Some(previous) = self
                    .catalog_sessions
                    .iter_mut()
                    .find(|s| s.path == session.path)
                {
                    *previous = session.clone();
                } else {
                    self.catalog_sessions.push(session.clone());
                }
                let _ = self.event_tx.send(RuntimeEvent::SessionUpdated(session));
            }
            event @ RuntimeEvent::SystemNotification { .. } => {
                let _ = self.event_tx.send(event);
            }
            RuntimeEvent::Snapshot { snapshot, .. } => {
                let mut snapshot = snapshot;
                // A fast catalog can finish before the actor's first snapshot.
                if snapshot.models.is_empty()
                    && let Some(actor) = self.actors.get(&key)
                    && let Some(command) = self
                        .configurations
                        .catalog_command(&snapshot.harness, &snapshot.project)
                {
                    actor.send(command);
                }
                // Publish identity even when the actor finishes starting in the background.
                if let Some(target) = snapshot.session_target()
                    && self
                        .latest
                        .get(&key)
                        .and_then(|previous| previous.session_target())
                        != Some(target.clone())
                {
                    let _ = self.event_tx.send(RuntimeEvent::SessionTarget(target));
                }
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
                    if let Some(dialogs) = self.active_dialogs.get_mut(&key) {
                        dialogs.retain(|request| {
                            request.dialog_id().is_some_and(agents::is_child_input_id)
                        });
                        if dialogs.is_empty() {
                            self.active_dialogs.remove(&key);
                            self.needs_input.remove(&key);
                        }
                    } else {
                        self.needs_input.remove(&key);
                    }
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
            RuntimeEvent::ExtensionUi {
                request,
                system_notification_target,
                ..
            } => {
                let system_notification_target = system_notification_target.or_else(|| {
                    self.latest
                        .get(&key)
                        .and_then(|snapshot| notification_target(snapshot))
                });
                if let Some(notification) = interaction_notification(
                    &request,
                    self.active_dialogs
                        .get(&key)
                        .map(Vec::as_slice)
                        .unwrap_or_default(),
                    system_notification_target,
                ) {
                    let _ = self.event_tx.send(notification);
                }
                if request.gpui_system_notification().is_some() {
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
            event @ RuntimeEvent::PromptResult { .. } => {
                let _ = self.event_tx.send(event);
            }
            RuntimeEvent::RefreshCatalog => {
                if let Some(catalog) = self.actors.get(&self.catalog_key) {
                    catalog.send(RuntimeCommand::RefreshSessions);
                }
            }
            event @ (RuntimeEvent::ImportPreview { .. }
            | RuntimeEvent::ImportPreviewFailed { .. }) => {
                let _ = self.event_tx.send(event);
            }
            mut event @ (RuntimeEvent::Sessions { .. } | RuntimeEvent::SessionsFailed { .. }) => {
                if key == self.catalog_key
                    && let RuntimeEvent::Sessions {
                        generation: next_generation,
                        all_sessions,
                        sessions,
                        ..
                    } = &mut event
                {
                    // SQLite stores archive and metadata, while app events own live status.
                    let running: HashSet<_> = self
                        .catalog_sessions
                        .iter()
                        .filter(|session| session.is_running)
                        .map(|session| &session.path)
                        .collect();
                    for session in all_sessions.iter_mut().chain(sessions.iter_mut()) {
                        session.is_running = running.contains(&session.path);
                    }
                    self.catalog_generation = *next_generation;
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
                        let _ = self.event_tx.send(*event);
                    }
                    SupervisorSessionAction::RefreshCatalog => {
                        if let Some(catalog) = self.actors.get(&self.catalog_key) {
                            catalog.send(RuntimeCommand::RefreshSessions);
                        }
                    }
                }
            }
            RuntimeEvent::Stopped
            | RuntimeEvent::SessionTarget(_)
            | RuntimeEvent::SessionMoved { .. }
            | RuntimeEvent::SessionDeleted { .. }
            | RuntimeEvent::SessionStatus { .. }
            | RuntimeEvent::SessionReset { .. }
            | RuntimeEvent::HistoryReset { .. } => {}
        }
    }
}
