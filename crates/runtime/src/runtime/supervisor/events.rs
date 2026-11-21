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

    pub(super) fn handle_actor_event(&mut self, key: String, event: RuntimeEvent) {
        match event {
            RuntimeEvent::SessionMetadata(metadata) => {
                if let Some(catalog) = self.actors.get(&self.catalog_key) {
                    catalog.send(RuntimeCommand::UpdateSessionMetadata(metadata));
                }
            }
            RuntimeEvent::SessionUpdated(mut session) => {
                if let Some(running) = self.live_catalog_running_for(
                    &session,
                    self.catalog_sessions
                        .at_path(&session.path)
                        .any(|row| !row.same_record(&session)),
                    &self.catalog_sessions,
                ) {
                    session.is_running = running;
                }
                if self.catalog_sessions.upsert_in_place(session.clone()) {
                    let _ = self.event_tx.send(RuntimeEvent::SessionUpdated(session));
                }
            }
            RuntimeEvent::SessionArchiveResult { request, rows } => {
                if let Ok(rows) = &rows {
                    let requested_path = match &request.target {
                        ArchiveTarget::Native(target) => Some(target.path.as_path()),
                        ArchiveTarget::Path(path) => Some(path.as_path()),
                        ArchiveTarget::App(_) | ArchiveTarget::Draft(_) => None,
                    };
                    for row in rows {
                        let index = self
                            .catalog_sessions
                            .apply_archive_result(row, requested_path, request.archived)
                            .unwrap_or_else(|| {
                                // A draft may bind before its first catalog update.
                                let mut session = row.clone();
                                if let Some(running) = self.live_catalog_running_for(
                                    &session,
                                    self.catalog_sessions.by_path(&session.path).is_some(),
                                    &self.catalog_sessions,
                                ) {
                                    session.is_running = running;
                                }
                                self.catalog_sessions.push(session.clone());
                                let _ = self.event_tx.send(RuntimeEvent::SessionUpdated(session));
                                self.catalog_sessions.len() - 1
                            });
                        reconcile_live_session_documents(
                            std::slice::from_ref(&self.catalog_sessions[index]),
                            &self.catalog_sessions,
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
                }
                let _ = self
                    .event_tx
                    .send(RuntimeEvent::SessionArchiveResult { request, rows });
            }
            RuntimeEvent::WorkerSnapshot(snapshot) => {
                if let Some(session) =
                    catalog::session_for_worker_snapshot(&self.catalog_sessions, &snapshot)
                {
                    let activity = AgentActivity::from_worker_status(
                        session,
                        snapshot.status,
                        snapshot.output.is_some(),
                    );
                    let _ = self.event_tx.send(RuntimeEvent::AgentActivityUpdated {
                        key: session.activity_key(),
                        activity,
                    });
                } else if let Some(catalog) = self.actors.get(&self.catalog_key) {
                    // The worker can bind before its session or its parent reaches this catalog.
                    catalog.send(RuntimeCommand::RefreshSessions);
                }
            }
            event @ (RuntimeEvent::AgentActivityUpdated { .. }
            | RuntimeEvent::DraftPromptCommitted { .. }
            | RuntimeEvent::DraftIdentitySaved { .. }
            | RuntimeEvent::SystemNotification { .. }
            | RuntimeEvent::TurnCompletedNotification { .. }) => {
                let _ = self.event_tx.send(event);
            }
            RuntimeEvent::Snapshot { snapshot, .. } => {
                let mut snapshot = snapshot;
                if let Some(actor) = self.actors.get(&key)
                    && let Some(command) =
                        self.configurations.catalog_command_for_snapshot(&snapshot)
                {
                    actor.send(command);
                }
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
                                || matches!(request, ExtensionUiRequest::Questions(_))
                        });
                        if !dialogs.iter().any(ExtensionUiRequest::is_blocking) {
                            self.needs_input.remove(&key);
                        }
                        if dialogs.is_empty() {
                            self.active_dialogs.remove(&key);
                            self.needs_input.remove(&key);
                        }
                    } else {
                        self.needs_input.remove(&key);
                    }
                }
                if let Some(running) = live_catalog_running(&snapshot)
                    && let Some(path) = snapshot.live_session.as_deref()
                    && let Some(record) = resident_status_record(
                        &self.published_statuses,
                        &self.catalog_sessions,
                        &key,
                        Some(&snapshot),
                        Some(path),
                    )
                    && let Some(session) =
                        self.catalog_sessions.set_record_running(&record, running)
                {
                    let _ = self
                        .event_tx
                        .send(RuntimeEvent::SessionUpdated(session.clone()));
                }
                let status = session_status(&self.needs_input, &key, &snapshot);
                publish_resident_status(
                    &self.event_tx,
                    &mut self.published_statuses,
                    &self.catalog_sessions,
                    &key,
                    Some(&snapshot),
                    status,
                );
                if let Some(path) = snapshot
                    .live_session
                    .clone()
                    .or_else(|| snapshot.selected_session.clone())
                    && let Some(harness) = snapshot.harness
                {
                    self.actor_paths.insert(path, harness, key.clone());
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
                    if !dialogs.iter().any(ExtensionUiRequest::is_blocking) {
                        self.needs_input.remove(&key);
                    }
                    if dialogs.is_empty() {
                        self.active_dialogs.remove(&key);
                        self.needs_input.remove(&key);
                    }
                }
                if let Some(snapshot) = self.latest.get(&key) {
                    let status = session_status(&self.needs_input, &key, snapshot);
                    publish_resident_status(
                        &self.event_tx,
                        &mut self.published_statuses,
                        &self.catalog_sessions,
                        &key,
                        Some(snapshot),
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
                    if request.is_blocking() {
                        self.needs_input.insert(key.clone());
                    }
                    publish_resident_status(
                        &self.event_tx,
                        &mut self.published_statuses,
                        &self.catalog_sessions,
                        &key,
                        self.latest.get(&key).map(AsRef::as_ref),
                        self.latest
                            .get(&key)
                            .map(|snapshot| session_status(&self.needs_input, &key, snapshot))
                            .unwrap_or(if request.is_blocking() {
                                RunStatus::NeedsInput
                            } else {
                                RunStatus::Working
                            }),
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
                    let mut next = sessions::SessionCatalog::from(all_sessions.clone());
                    for row in all_sessions.iter_mut().chain(sessions.iter_mut()) {
                        let shared = next.has_shared_path(&row.path);
                        let running = self
                            .live_catalog_running_for(row, shared, &next)
                            .unwrap_or_else(|| {
                                SessionStatusRecord::for_row(row)
                                    .resolve(&self.catalog_sessions)
                                    .is_some_and(|previous| previous.is_running)
                            });
                        if row.is_running != running {
                            next.set_record_running(&SessionStatusRecord::for_row(row), running);
                            row.is_running = running;
                        }
                    }
                    self.catalog_generation = *next_generation;
                    self.catalog_sessions = next;
                    reconcile_live_session_documents(
                        all_sessions,
                        &self.catalog_sessions,
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
            | RuntimeEvent::SelectionRouted(_)
            | RuntimeEvent::SelectionRejected(_)
            | RuntimeEvent::SessionTarget(_)
            | RuntimeEvent::SessionMoved { .. }
            | RuntimeEvent::SessionDeleted { .. }
            | RuntimeEvent::SessionStatus { .. }
            | RuntimeEvent::SessionReset { .. }
            | RuntimeEvent::AccountsChanged(_)
            | RuntimeEvent::HistoryReset { .. } => {}
        }
    }

    fn live_catalog_running_for(
        &self,
        row: &SessionSummary,
        shared: bool,
        catalog: &sessions::SessionCatalog,
    ) -> Option<bool> {
        let path = &row.path;
        let snapshot = if shared {
            self.latest.iter().find_map(|(actor, snapshot)| {
                if snapshot.live_session.as_ref() != Some(path) {
                    return None;
                }
                let record = resident_status_record(
                    &self.published_statuses,
                    catalog,
                    actor,
                    Some(snapshot),
                    Some(path),
                )?;
                record.matches(row).then_some(snapshot)
            })
        } else {
            self.actor_paths
                .get(path, row.harness)
                .and_then(|key| self.latest.get(key))
                .filter(|snapshot| {
                    snapshot.live_session.as_ref() == Some(path) && snapshot.matches_lone_row(row)
                })
        }?;
        live_catalog_running(snapshot)
    }
}

fn live_catalog_running(snapshot: &RuntimeSnapshot) -> Option<bool> {
    if snapshot.history_preview {
        match snapshot.live_status {
            RunStatus::Working
            | RunStatus::Compacting
            | RunStatus::Retrying
            | RunStatus::NeedsInput => Some(true),
            RunStatus::Done | RunStatus::Failed | RunStatus::Stopped => Some(false),
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

fn session_status(
    needs_input: &HashSet<String>,
    key: &str,
    snapshot: &RuntimeSnapshot,
) -> RunStatus {
    if needs_input.contains(key) {
        RunStatus::NeedsInput
    } else if snapshot.history_preview && snapshot.live_status != RunStatus::Invalid {
        snapshot.live_status
    } else {
        semantic_status(snapshot)
    }
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
