use super::*;
use crate::agents::Backend;
use std::path::Path;

impl Supervisor {
    pub(super) fn drain_configuration_updates(&mut self) {
        while let Ok((harness, profile_id, project, result)) = self.configuration_rx.try_recv() {
            match result {
                Ok(catalog) => {
                    self.configurations.set_catalog_for_profile(
                        harness,
                        profile_id.clone(),
                        project.clone(),
                        catalog.clone(),
                    );
                    if cache_configuration_catalog(
                        &mut self.configuration_catalogs,
                        harness,
                        profile_id.clone(),
                        project.clone(),
                        catalog,
                    ) && let Some(state) = self.catalog_state.as_ref()
                    {
                        let _ = state.with(|store| {
                            store.save_configuration_catalogs(&self.configuration_catalogs)
                        });
                    }
                }
                Err(error) => {
                    self.configuration_requests.remove(&(
                        harness,
                        profile_id.clone(),
                        project.clone(),
                    ));
                    zlog::warn!("Failed to refresh {harness} catalog: {error}");
                    self.configurations.set_catalog_error(
                        harness,
                        profile_id.clone(),
                        project.clone(),
                        error,
                    );
                }
            }
            self.publish_configuration_snapshots(harness, profile_id.as_deref(), &project);
        }
    }

    pub(super) fn publish_configuration_snapshots(
        &mut self,
        harness: Backend,
        profile_id: Option<&str>,
        project: &Path,
    ) {
        for (key, snapshot) in &mut self.latest {
            if snapshot.harness == Some(harness)
                && snapshot.profile_id.as_deref() == profile_id
                && snapshot.project == project
            {
                if let Some(actor) = self.actors.get(key)
                    && let Some(command) = self
                        .configurations
                        .catalog_command_for_profile(harness, profile_id, project)
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
            RuntimeEvent::SessionUpdated(mut session) => {
                if let Some(running) = self.live_catalog_running_for(&session.path) {
                    // A catalog reply can describe an earlier point in the
                    // same turn. The live actor owns its current run state.
                    session.is_running = running;
                }
                let changed = if let Some(previous) = self
                    .catalog_sessions
                    .iter_mut()
                    .find(|s| s.path == session.path)
                {
                    if *previous == session {
                        false
                    } else {
                        *previous = session.clone();
                        true
                    }
                } else {
                    self.catalog_sessions.push(session.clone());
                    true
                };
                if changed {
                    let _ = self.event_tx.send(RuntimeEvent::SessionUpdated(session));
                }
            }
            RuntimeEvent::AgentActivityUpdated(activity) => {
                let _ = self
                    .event_tx
                    .send(RuntimeEvent::AgentActivityUpdated(activity));
            }
            event @ (RuntimeEvent::SystemNotification { .. }
            | RuntimeEvent::TurnCompletedNotification { .. }) => {
                let _ = self.event_tx.send(event);
            }
            RuntimeEvent::Snapshot { snapshot, .. } => {
                let mut snapshot = snapshot;
                // A fast catalog can finish before the actor's first snapshot.
                // Send only missing or stale state: capability-only Pi catalogs
                // can have no models, so an empty-model test alone loops forever.
                if let Some(actor) = self.actors.get(&key)
                    && let Some(command) =
                        self.configurations.catalog_command_for_snapshot(&snapshot)
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
                    && let Some(harness) = snapshot.harness
                    && cache_configuration_catalog(
                        &mut self.configuration_catalogs,
                        harness,
                        snapshot.profile_id.clone(),
                        snapshot.project.clone(),
                        crate::agents::ConfigurationCatalog {
                            models: snapshot.models.clone(),
                            efforts: snapshot.thinking_levels.clone(),
                            sandbox_adapter: snapshot.sandbox_adapter.clone(),
                        },
                    )
                    && let Some(state) = self.catalog_state.as_ref()
                {
                    let _ = state.with(|store| {
                        store.save_configuration_catalogs(&self.configuration_catalogs)
                    });
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
                if let Some(running) = live_catalog_running(&snapshot)
                    && let Some(path) = snapshot.live_session.as_ref()
                    && let Some(session) = self
                        .catalog_sessions
                        .iter_mut()
                        .find(|session| &session.path == path)
                    && session.is_running != running
                {
                    session.is_running = running;
                    let _ = self
                        .event_tx
                        .send(RuntimeEvent::SessionUpdated(session.clone()));
                }
                let status = session_status(&self.needs_input, &key, &snapshot);
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
                    self.selected_project = snapshot.project.clone();
                    self.selected_session = snapshot
                        .live_session
                        .clone()
                        .or_else(|| snapshot.selected_session.clone());
                    let _ = self.event_tx.send(RuntimeEvent::Snapshot {
                        generation: self.generation,
                        snapshot,
                    });
                }
            }
            RuntimeEvent::ExtensionUiDismissed { id, .. } => {
                if let Some(dialogs) = self.active_dialogs.get_mut(&key) {
                    dialogs.retain(|request| request.dialog_id() != Some(id.as_str()));
                    if dialogs.is_empty() {
                        self.active_dialogs.remove(&key);
                        self.needs_input.remove(&key);
                    }
                }
                if let Some(snapshot) = self.latest.get(&key) {
                    let status = session_status(&self.needs_input, &key, snapshot);
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
                }
                if key == self.selected {
                    let _ = self.event_tx.send(RuntimeEvent::ExtensionUiDismissed {
                        generation: self.generation,
                        id,
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
                        session.is_running = self
                            .live_catalog_running_for(&session.path)
                            .unwrap_or_else(|| running.contains(&session.path));
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
                        &self.host,
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

    fn live_catalog_running_for(&self, path: &Path) -> Option<bool> {
        self.actor_paths
            .get(path)
            .and_then(|key| self.latest.get(key))
            .filter(|snapshot| snapshot.live_session.as_deref() == Some(path))
            .and_then(|snapshot| live_catalog_running(snapshot))
    }
}

fn live_catalog_running(snapshot: &RuntimeSnapshot) -> Option<bool> {
    if snapshot.history_preview {
        match snapshot.live_status.as_str() {
            "Working" | "Compacting" | "Retrying" | "Needs input" => Some(true),
            "Done" | "Failed" | "Stopped" => Some(false),
            _ => None,
        }
    } else if snapshot.conversation.running
        || snapshot.conversation.compacting
        || snapshot.conversation.retrying
        || snapshot.conversation.settled
        || matches!(
            snapshot.status.as_str(),
            "Done" | "Failed" | "Stopped" | "Ready" | "Command failed"
        )
    {
        Some(
            snapshot.conversation.running
                || snapshot.conversation.compacting
                || snapshot.conversation.retrying,
        )
    } else {
        None
    }
}

fn session_status<'a>(
    needs_input: &HashSet<String>,
    key: &str,
    snapshot: &'a RuntimeSnapshot,
) -> &'a str {
    if needs_input.contains(key) {
        "Needs input"
    } else if snapshot.history_preview && !snapshot.live_status.is_empty() {
        &snapshot.live_status
    } else {
        semantic_status(snapshot)
    }
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
