use super::*;
use farcaster_sessions::archived_root_family_for_path;
use std::path::Path;

impl Supervisor {
    fn stop_session_family_work(
        &mut self,
        path: &Path,
        key: Option<&sessions::SessionKey>,
        archive: bool,
    ) -> Result<(), String> {
        let requested = match key {
            Some(key) => self
                .catalog_sessions
                .by_key(key)
                .filter(|row| row.path == path),
            None => self.catalog_sessions.by_path(path),
        }
        .ok_or("The session family is no longer available")?;
        let target = sessions::AppSessionId::new(requested.app_session_id)
            .map(ArchiveTarget::App)
            .unwrap_or_else(|| ArchiveTarget::Native(requested.target()));
        self.stop_family_work(
            Some(requested.catalog_key()),
            requested.project.clone(),
            target,
            None,
            archive,
        )
    }

    fn stop_and_archive_draft(
        &mut self,
        draft: &str,
        identity: &DraftArchiveIdentity,
    ) -> Result<(), String> {
        let id = match identity {
            DraftArchiveIdentity::Saved(id) => *id,
            DraftArchiveIdentity::Pending(saved) => {
                saved.recv_blocking().map_err(|e| e.to_string())??
            }
        };
        let (project, row) = self
            .catalog_state
            .as_ref()
            .ok_or("Session state is unavailable")?
            .with(|store| store.draft_archive_session(id, draft))?;
        let _ = self.event_tx.send(RuntimeEvent::DraftArchiveIdentity {
            draft: draft.to_owned(),
            app_session_id: id,
        });
        let key = row.as_ref().map(SessionSummary::catalog_key);
        if let Some(row) = row
            && self.catalog_sessions.by_app_id(id.get()).is_none()
        {
            self.handle_actor_event(self.catalog_key.clone(), RuntimeEvent::SessionUpdated(row));
        }
        self.stop_family_work(
            key,
            project,
            ArchiveTarget::App(id),
            Some((id, sessions::draft_target(draft))),
            true,
        )
    }

    fn stop_family_work(
        &mut self,
        key: Option<sessions::SessionKey>,
        project: PathBuf,
        archive_target: ArchiveTarget,
        draft_actor: Option<(sessions::AppSessionId, String)>,
        archive: bool,
    ) -> Result<(), String> {
        let catalog = &self.catalog_sessions;
        let family = match key.as_ref() {
            Some(key) => {
                let root = catalog
                    .by_key(key)
                    .and_then(|row| catalog.root_for_row(row))
                    .ok_or("The session family is no longer available")?;
                std::iter::once(root)
                    .chain(catalog.descendants(root).into_iter().map(|(row, _)| row))
                    .collect::<Vec<_>>()
            }
            None => Vec::new(),
        };
        let family_paths = family
            .iter()
            .map(|session| session.path.clone())
            .collect::<HashSet<_>>();
        let prior_actor_failures = family_paths
            .iter()
            .filter_map(|path| self.failed_actor_shutdowns.get(path))
            .chain(
                family
                    .iter()
                    .filter_map(|row| sessions::AppSessionId::new(row.app_session_id))
                    .chain(draft_actor.as_ref().map(|(id, _)| *id))
                    .filter_map(|id| self.failed_draft_shutdowns.get(&id)),
            )
            .cloned()
            .collect::<Vec<_>>();
        if !prior_actor_failures.is_empty() {
            return Err(format!(
                "Could not confirm the whole session family stopped: {}",
                prior_actor_failures.join("; ")
            ));
        }
        let mut worker_keys = family
            .iter()
            .map(|session| session.key())
            .collect::<Vec<_>>();
        if worker_keys.is_empty()
            && let Some((id, _)) = &draft_actor
        {
            worker_keys.push(sessions::SessionKey::App(*id));
        }
        self.require_current_family_keys(&project, &family)?;
        if let Err(message) = self
            .host
            .stop_session_family_workers(&project, &worker_keys)
        {
            return Err(format!(
                "Could not stop the whole session family: {message}"
            ));
        }
        if let Err(message) = self.require_current_family_keys(&project, &family) {
            if let Err(cleanup) = self
                .host
                .finish_session_family_worker_stop(&project, &worker_keys)
            {
                return Err(format!(
                    "{message}; could not release the worker stop fence: {cleanup}"
                ));
            }
            return Err(message);
        }
        let family_actor_keys = family
            .iter()
            .map(|row| documents::catalog_actor_key(row, catalog, &self.actor_paths, &self.latest))
            .chain(draft_actor.as_ref().map(|(_, actor)| actor.clone()))
            .filter(|key| {
                key != &self.catalog_key
                    && (self.actors.contains_key(key) || self.latest.contains_key(key))
            })
            .collect::<HashSet<_>>();
        // Legacy catalog paths can retain a symlink. The worker pool needs
        // normalized keys, while these rows still use their stored locators.
        let family_keys = family
            .iter()
            .map(|row| row.catalog_key())
            .collect::<HashSet<_>>();
        let shared = family
            .iter()
            .filter(|row| catalog.has_shared_path(&row.path))
            .map(|row| row.path.clone())
            .collect::<HashSet<_>>();
        let mut stopped_sessions = Vec::new();
        let mut actor_stop_failures = Vec::new();
        for key in &family_actor_keys {
            if let Some(actor) = self.actors.remove(key) {
                actor.send(RuntimeCommand::Shutdown);
                if let Err(error) = actor.join() {
                    let message = format!("{key}: {error}");
                    for path in self
                        .actor_paths
                        .iter()
                        .filter_map(|(path, actor_key)| (actor_key == key).then_some(path))
                    {
                        self.failed_actor_shutdowns
                            .insert(path.clone(), message.clone());
                    }
                    if let Some((id, actor)) = &draft_actor
                        && actor == key
                    {
                        self.failed_draft_shutdowns.insert(*id, message.clone());
                    }
                    actor_stop_failures.push(message);
                }
            }
            let session = self.latest.get(key).and_then(|snapshot| {
                snapshot
                    .live_session
                    .clone()
                    .or_else(|| snapshot.selected_session.clone())
            });
            let record = resident_status_record(
                &self.published_statuses,
                &self.catalog_sessions,
                key,
                self.latest.get(key).map(AsRef::as_ref),
                session.as_deref(),
            );
            stopped_sessions.push((key.clone(), session, record));
            self.latest.remove(key);
            self.last_touch.remove(key);
            self.pending_extensions.remove(key);
            self.active_dialogs.remove(key);
            self.needs_input.remove(key);
            self.interacted.remove(key);
            self.published_statuses.remove(key);
        }
        self.document_revisions
            .retain(|key, _| !family_actor_keys.contains(key));
        self.actor_paths
            .retain(|_, key| !family_actor_keys.contains(key));
        if family_actor_keys.contains(&self.selected) {
            self.selected = self.catalog_key.clone();
        }
        if !actor_stop_failures.is_empty() {
            let _ = self
                .host
                .finish_session_family_worker_stop(&project, &worker_keys);
            return Err(format!(
                "Could not confirm the whole session family stopped: {}",
                actor_stop_failures.join("; ")
            ));
        }
        for (target, session, record) in stopped_sessions {
            if record.is_none() && session.as_ref().is_some_and(|path| shared.contains(path)) {
                continue;
            }
            let _ = self.event_tx.send(RuntimeEvent::SessionStatus {
                target,
                session,
                record,
                status: RunStatus::Stopped,
            });
        }
        for session in self.catalog_sessions.iter_mut() {
            if family_keys.contains(&session.catalog_key()) {
                session.is_running = false;
                if shared.contains(&session.path) {
                    let _ = self
                        .event_tx
                        .send(RuntimeEvent::SessionUpdated(session.clone()));
                    let record = SessionStatusRecord::for_row(session);
                    let _ = self.event_tx.send(RuntimeEvent::SessionStatus {
                        target: record.status_key(),
                        record: Some(record),
                        session: Some(session.path.clone()),
                        status: RunStatus::Stopped,
                    });
                    continue;
                }
                let _ = self.event_tx.send(RuntimeEvent::SessionStatus {
                    target: format!("session:{}", session.path.display()),
                    record: Some(SessionStatusRecord::for_row(session)),
                    session: Some(session.path.clone()),
                    status: RunStatus::Stopped,
                });
            }
        }
        if archive && let Ok(snapshots) = self.host.worker_snapshots() {
            let stopped = self
                .catalog_sessions
                .iter()
                .filter(|session| family_keys.contains(&session.catalog_key()))
                .cloned()
                .collect::<Vec<_>>();
            for (key, activity) in catalog::worker_activities(&stopped, snapshots) {
                let _ = self
                    .event_tx
                    .send(RuntimeEvent::AgentActivityUpdated { key, activity });
            }
        }
        if let Err(message) = self
            .host
            .finish_session_family_worker_stop(&project, &worker_keys)
        {
            return Err(format!(
                "Session family stopped, but its worker stop fence failed: {message}"
            ));
        }
        if archive {
            let catalog = self
                .actors
                .get(&self.catalog_key)
                .ok_or("Session family stopped, but the session catalog is unavailable")?;
            catalog
                .try_send(RuntimeCommand::SetSessionArchived {
                    request: ArchiveRequest {
                        target: archive_target,
                        archived: true,
                        sequence: None,
                    },
                    draft_saved: None,
                })
                .map_err(|message| {
                    format!("Session family stopped, but could not be archived: {message}")
                })?;
        } else if let Some(catalog) = self.actors.get(&self.catalog_key) {
            catalog.send(RuntimeCommand::RefreshSessions);
        }
        Ok(())
    }

    fn require_current_family_keys(
        &self,
        project: &Path,
        rows: &[&SessionSummary],
    ) -> Result<(), String> {
        if rows.iter().all(|row| row.app_session_id <= 0) {
            return Ok(());
        }
        self.catalog_state.as_ref().ok_or("Session state is unavailable")?.with(|store| {
            if store.contains_project_sessions(project, rows)? { Ok(()) }
            else { Err("The session identity changed; refresh the session list before stopping its family".into()) }
        })
    }

    fn discard_family_queue(&mut self, path: &Path) -> Result<(), String> {
        let family = session_family_for_path(&self.catalog_sessions, path)
            .ok_or_else(|| "The session family is no longer available".to_owned())?;
        let paths = family
            .iter()
            .map(|session| crate::sessions::normalize_session_path(&session.path))
            .collect::<HashSet<_>>();
        let state = self.host.state_store()?;
        state.with(|store| {
            let queued = store.queued_prompts()?;
            let ids = queued
                .iter()
                .filter(|prompt| {
                    prompt.session.as_ref().is_some_and(|session| {
                        paths.contains(&crate::sessions::normalize_session_path(session))
                    })
                })
                .map(|prompt| prompt.id)
                .collect::<Vec<_>>();
            store.cancel_queued_prompts(&ids)
        })
    }

    fn restore_selected_session_after_failed_move(&mut self, root: &Path) {
        let Some(selected_path) = self.selected_session.as_ref() else {
            return;
        };
        if self.selected != self.catalog_key
            || self.failed_actor_shutdowns.contains_key(selected_path)
            || !selected_path.is_file()
        {
            return;
        }
        let Some(session) = session_family_for_path(&self.catalog_sessions, root)
            .filter(|family| family[0].path == root)
            .and_then(|family| {
                family
                    .into_iter()
                    .find(|session| session.path == *selected_path)
            })
        else {
            return;
        };
        let session = session.clone();
        let key = self
            .actor_paths
            .session_key(&session.path, session.harness, &self.latest);
        let actor = SessionRuntimeHandle::spawn(
            session.project.clone(),
            self.process_command.clone(),
            false,
            Some(session.harness),
            self.supervisor_thread.clone(),
            self.host.clone(),
        );
        send_configured_command(
            &actor,
            RuntimeCommand::SelectSession {
                path: session.path.clone(),
                harness: session.harness,
                session_id: session.id,
                project: session.project.clone(),
                key: None,
                request: None,
            },
            &self.configurations,
            saved_access_mode(
                self.catalog_state.as_ref(),
                &session.path,
                Some(session.harness),
            ),
            session.profile_id.as_deref(),
        );
        self.generation = self.generation.saturating_add(1);
        self.selected = key.clone();
        self.selected_project = session.project;
        self.actor_paths
            .insert(session.path, session.harness, key.clone());
        self.actors.insert(key, actor);
    }

    pub(super) fn handle_session_family_command(&mut self, command: &RuntimeCommand) -> bool {
        if let RuntimeCommand::StopAndArchiveDraft { draft, identity } = command {
            if let Err(message) = self.stop_and_archive_draft(draft, identity) {
                let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                    generation: self.catalog_generation,
                    message,
                });
            }
            return true;
        }
        if let RuntimeCommand::StopSessionFamily { path, key } = command {
            if let Err(message) = self.stop_session_family_work(path, key.as_ref(), true) {
                let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                    generation: self.catalog_generation,
                    message,
                });
            }
            return true;
        }
        if let RuntimeCommand::StopAndDeleteSessionFamily { path } = command {
            let archived = archived_root_family_for_path(&self.catalog_sessions, path).is_some();
            let result = if archived {
                self.stop_session_family_work(path, None, false)
                    .and_then(|()| self.discard_family_queue(path))
            } else {
                Err("Only an archived root session can be deleted".to_owned())
            };
            if let Err(message) = result {
                let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                    generation: self.catalog_generation,
                    message,
                });
            } else {
                self.handle_session_family_command(&RuntimeCommand::DeleteSessionFamily {
                    path: path.clone(),
                });
            }
            return true;
        }
        if let RuntimeCommand::StopAndMoveSession {
            path,
            target_project,
        } = command
        {
            let result = session_family_for_path(&self.catalog_sessions, path)
                .ok_or_else(|| "The session is no longer available to move".to_owned())
                .and_then(|family| {
                    if family[0].path != *path {
                        return Err("Only a root session can be moved".to_owned());
                    }
                    let owned = family
                        .iter()
                        .map(|session| (*session).clone())
                        .collect::<Vec<_>>();
                    agents::validate_session_move(&owned)
                })
                .and_then(|()| self.stop_session_family_work(path, None, false));
            if let Err(message) = result {
                let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                    generation: self.catalog_generation,
                    message,
                });
            } else if let Err(message) = self.discard_family_queue(path) {
                self.restore_selected_session_after_failed_move(path);
                let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                    generation: self.catalog_generation,
                    message,
                });
            } else {
                self.handle_session_family_command(&RuntimeCommand::MoveSession {
                    path: path.clone(),
                    target_project: target_project.clone(),
                });
            }
            return true;
        }
        if let RuntimeCommand::DeleteSessionFamily { path } = &command {
            let result = (|| {
                self.stop_session_family_work(path, None, false)?;
                self.discard_family_queue(path)?;
                let family = session_family_for_path(&self.catalog_sessions, path)
                    .ok_or_else(|| "The session is no longer available to delete".to_owned())?;
                let targets = family
                    .iter()
                    .map(|session| session.target())
                    .collect::<Vec<_>>();
                let family_paths = family
                    .iter()
                    .map(|session| session.path.clone())
                    .collect::<HashSet<_>>();
                let family_actor_keys = self
                    .actor_paths
                    .iter()
                    .filter(|(path, key)| family_paths.contains(*path) && *key != &self.catalog_key)
                    .map(|(_, key)| key.clone())
                    .collect::<HashSet<_>>();

                for key in &family_actor_keys {
                    if let Some(actor) = self.actors.remove(key) {
                        actor.send(RuntimeCommand::Shutdown);
                        let _ = actor.join();
                    }
                    self.latest.remove(key);
                    self.last_touch.remove(key);
                    self.pending_extensions.remove(key);
                    self.active_dialogs.remove(key);
                    self.needs_input.remove(key);
                    self.interacted.remove(key);
                    self.published_statuses.remove(key);
                }
                self.document_revisions
                    .retain(|_, (path, _, _)| !family_paths.contains(path));
                self.actor_paths
                    .retain(|path, _| !family_paths.contains(path));
                if family_actor_keys.contains(&self.selected) {
                    self.selected = self.catalog_key.clone();
                    self.generation = self.generation.saturating_add(1);
                }
                let state = self.host.state_store()?;
                let paths = family_paths.iter().cloned().collect::<Vec<_>>();
                let leftovers =
                    agents::delete_session_family_with_config(&self.process_command, &targets)?;
                let state_warning = state
                    .with(|store| sessions::delete_state(store, &paths))
                    .err();
                Ok((family_paths, leftovers, state_warning))
            })();
            match result {
                Ok((paths, leftovers, state_warning)) => {
                    let _ = self.event_tx.send(RuntimeEvent::SessionDeleted {
                        generation: self.generation,
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
                        let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                            generation: self.catalog_generation,
                            message: format!("Session deleted, but {}", warnings.join("; ")),
                        });
                    }
                    if let Some(catalog) = self.actors.get(&self.catalog_key) {
                        catalog.send(RuntimeCommand::RefreshSessions);
                    }
                }
                Err(message) => {
                    let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                        generation: self.catalog_generation,
                        message,
                    });
                    if let Some(catalog) = self.actors.get(&self.catalog_key) {
                        catalog.send(RuntimeCommand::RefreshSessions);
                    }
                }
            }
            return true;
        }
        if let RuntimeCommand::MoveSession {
            path,
            target_project,
        } = &command
        {
            let result = (|| {
                let family = session_family_for_path(&self.catalog_sessions, path)
                    .ok_or_else(|| "The session is no longer available to move".to_owned())?;
                let root = family[0];
                if root.path != *path {
                    return Err("Only a root session can be moved".to_owned());
                }
                let owned_family = family
                    .iter()
                    .map(|session| (*session).clone())
                    .collect::<Vec<_>>();
                agents::validate_session_move(&owned_family)?;
                if family.iter().any(|session| session.is_running) {
                    return Err("Wait for the session family to finish before moving it".to_owned());
                }
                let family_paths = family
                    .iter()
                    .map(|session| session.path.clone())
                    .collect::<HashSet<_>>();
                let family_actor_keys = self
                    .actor_paths
                    .iter()
                    .filter(|(path, key)| family_paths.contains(*path) && *key != &self.catalog_key)
                    .map(|(_, key)| key.clone())
                    .collect::<HashSet<_>>();
                if family_actor_keys.iter().any(|key| {
                    self.latest.get(key).is_some_and(|snapshot| {
                        session_actor_has_active_work(snapshot, self.needs_input.contains(key))
                    })
                }) {
                    return Err(
                        "Wait for the session family to become idle before moving it".to_owned(),
                    );
                }
                let state = self.host.state_store()?;
                let paths = family_paths.iter().cloned().collect::<Vec<_>>();
                if state.with(|store| agents::has_queued_prompts_for(store, &paths))? {
                    return Err(
                        "Send or remove pending messages before moving this session".to_owned()
                    );
                }
                for key in &family_actor_keys {
                    if let Some(actor) = self.actors.remove(key) {
                        actor.send(RuntimeCommand::Shutdown);
                        let _ = actor.join();
                    }
                    self.latest.remove(key);
                    self.last_touch.remove(key);
                    self.pending_extensions.remove(key);
                    self.active_dialogs.remove(key);
                    self.needs_input.remove(key);
                    self.interacted.remove(key);
                    self.published_statuses.remove(key);
                }
                self.document_revisions
                    .retain(|_, (path, _, _)| !family_paths.contains(path));
                self.actor_paths
                    .retain(|path, _| !family_paths.contains(path));
                let source_was_selected = family_actor_keys.contains(&self.selected);
                if source_was_selected {
                    self.selected = self.catalog_key.clone();
                    self.generation = self.generation.saturating_add(1);
                }
                let moved = agents::move_session_family_with_config(
                    &self.process_command,
                    &owned_family,
                    target_project,
                )?;
                let path_updates = moved
                    .paths
                    .iter()
                    .map(|(source, target)| (source.clone(), target.clone()))
                    .collect::<Vec<_>>();
                let state_warning = state
                    .with(|store| sessions::relocate_state(store, &path_updates, target_project))
                    .err();
                let mut target = owned_family[0].target();
                target.path = moved.root.clone();
                Ok((moved, target, state_warning))
            })();
            match result {
                Ok((moved, target, state_warning)) => {
                    let _ = self.event_tx.send(RuntimeEvent::SessionMoved {
                        target,
                        target_project: target_project.clone(),
                        paths: Arc::new(moved.paths),
                    });
                    if let Some(message) = state_warning {
                        let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                        generation: self.catalog_generation,
                        message: format!(
                            "Session moved, but its saved UI state could not be migrated: {message}"
                        ),
                    });
                    }
                    if let Some(catalog) = self.actors.get(&self.catalog_key) {
                        catalog.send(RuntimeCommand::RefreshSessions);
                    }
                }
                Err(message) => {
                    self.restore_selected_session_after_failed_move(path);
                    let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                        generation: self.catalog_generation,
                        message,
                    });
                    if let Some(catalog) = self.actors.get(&self.catalog_key) {
                        catalog.send(RuntimeCommand::RefreshSessions);
                    }
                }
            }
            return true;
        }
        false
    }
}

fn session_actor_has_active_work(snapshot: &RuntimeSnapshot, needs_input: bool) -> bool {
    snapshot.conversation.running
        || snapshot.conversation.compacting
        || snapshot.conversation.retrying
        || needs_input
}

#[cfg(test)]
#[path = "family_commands_tests.rs"]
mod tests;
