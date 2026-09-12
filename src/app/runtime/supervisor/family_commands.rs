use super::*;

impl Supervisor {
    pub(super) fn handle_session_family_command(&mut self, command: &RuntimeCommand) -> bool {
        if let RuntimeCommand::StopSessionFamily { path } = &command {
            if let Some(family) = session_family_for_path(&self.catalog_sessions, path) {
                let family_paths = family
                    .iter()
                    .map(|session| session.path.clone())
                    .collect::<HashSet<_>>();
                let prior_actor_failures = family_paths
                    .iter()
                    .filter_map(|path| self.failed_actor_shutdowns.get(path))
                    .cloned()
                    .collect::<Vec<_>>();
                if !prior_actor_failures.is_empty() {
                    let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                        generation: self.catalog_generation,
                        message: format!(
                            "Could not confirm the whole session family stopped: {}",
                            prior_actor_failures.join("; ")
                        ),
                    });
                    return true;
                }
                let project = family[0].project.clone();
                let worker_paths = family
                    .iter()
                    .map(|session| (session.harness.clone(), session.path.clone()))
                    .collect::<Vec<_>>();
                if let Err(message) =
                    crate::app::mcp_server::stop_session_family_workers(&project, &worker_paths)
                {
                    let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                        generation: self.catalog_generation,
                        message: format!("Could not stop the whole session family: {message}"),
                    });
                    return true;
                }
                let family_actor_keys = self
                    .actor_paths
                    .iter()
                    .filter(|(path, key)| family_paths.contains(*path) && *key != &self.catalog_key)
                    .map(|(_, key)| key.clone())
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
                            actor_stop_failures.push(message);
                        }
                    }
                    let session = self.latest.get(key).and_then(|snapshot| {
                        snapshot
                            .live_session
                            .clone()
                            .or_else(|| snapshot.selected_session.clone())
                    });
                    stopped_sessions.push((key.clone(), session));
                    self.latest.remove(key);
                    self.last_touch.remove(key);
                    self.pending_extensions.remove(key);
                    self.active_dialogs.remove(key);
                    self.needs_input.remove(key);
                    self.interacted.remove(key);
                    self.published_statuses.remove(key);
                }
                self.document_revisions
                    .retain(|path, _| !family_paths.contains(path));
                self.actor_paths
                    .retain(|path, _| !family_paths.contains(path));
                if family_actor_keys.contains(&self.selected) {
                    self.selected = self.catalog_key.clone();
                }
                if !actor_stop_failures.is_empty() {
                    let _ = crate::app::mcp_server::finish_session_family_worker_stop(
                        &project,
                        &worker_paths,
                    );
                    let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                        generation: self.catalog_generation,
                        message: format!(
                            "Could not confirm the whole session family stopped: {}",
                            actor_stop_failures.join("; ")
                        ),
                    });
                    return true;
                }
                for (target, session) in stopped_sessions {
                    let _ = self.event_tx.send(RuntimeEvent::SessionStatus {
                        target,
                        session,
                        status: "Stopped".into(),
                    });
                }
                for session in &mut self.catalog_sessions {
                    if family_paths.contains(&session.path) {
                        session.is_running = false;
                        let _ = self.event_tx.send(RuntimeEvent::SessionStatus {
                            target: crate::app::composer::sessions::session_target(&session.path),
                            session: Some(session.path.clone()),
                            status: "Stopped".into(),
                        });
                    }
                }
                if let Err(message) = crate::app::mcp_server::finish_session_family_worker_stop(
                    &project,
                    &worker_paths,
                ) {
                    let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                        generation: self.catalog_generation,
                        message: format!(
                            "Session family stopped, but its worker stop fence failed: {message}"
                        ),
                    });
                    return true;
                }
                let archive_result = self
                    .catalog_state
                    .as_ref()
                    .ok_or_else(|| "Session state is unavailable".to_owned())
                    .and_then(|state| sessions::set_archived(state, path, true));
                if let Err(message) = archive_result {
                    let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                        generation: self.catalog_generation,
                        message: format!(
                            "Session family stopped, but could not be archived: {message}"
                        ),
                    });
                    return true;
                }
                if let Some(root) = self
                    .catalog_sessions
                    .iter_mut()
                    .find(|session| session.path == *path)
                {
                    root.archived = true;
                }
                if let Some(catalog) = self.actors.get(&self.catalog_key) {
                    catalog.send(RuntimeCommand::RefreshSessions);
                }
            }
            return true;
        }
        if let RuntimeCommand::DeleteSessionFamily { path } = &command {
            let result = (|| {
                let family = archived_root_family_for_path(&self.catalog_sessions, path)
                    .ok_or_else(|| "Only an archived root session can be deleted".to_owned())?;
                let targets = family
                    .iter()
                    .map(|session| session.target())
                    .collect::<Vec<_>>();
                if family.iter().any(|session| session.is_running) {
                    return Err(
                        "Wait for the session family to finish before deleting it".to_owned()
                    );
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
                        "Wait for the session family to become idle before deleting it".to_owned(),
                    );
                }
                let mut state = StateStore::open()?;
                let paths = family_paths.iter().cloned().collect::<Vec<_>>();
                if agents::has_queued_prompts_for(&state, &paths)? {
                    return Err(
                        "Send or remove queued prompts before deleting this session".to_owned()
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
                    .retain(|path, _| !family_paths.contains(path));
                self.actor_paths
                    .retain(|path, _| !family_paths.contains(path));
                if family_actor_keys.contains(&self.selected) {
                    self.selected = self.catalog_key.clone();
                    self.generation = self.generation.saturating_add(1);
                }
                let leftovers = agents::delete_session_family(&targets)?;
                let state_warning = sessions::delete_state(&mut state, &paths).err();
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
                let mut state = StateStore::open()?;
                let paths = family_paths.iter().cloned().collect::<Vec<_>>();
                if agents::has_queued_prompts_for(&state, &paths)? {
                    return Err(
                        "Send or remove queued prompts before moving this session".to_owned()
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
                    .retain(|path, _| !family_paths.contains(path));
                self.actor_paths
                    .retain(|path, _| !family_paths.contains(path));
                let source_was_selected = family_actor_keys.contains(&self.selected);
                if source_was_selected {
                    self.selected = self.catalog_key.clone();
                    self.generation = self.generation.saturating_add(1);
                }
                let moved = agents::move_session_family(&owned_family, target_project)?;
                let path_updates = moved
                    .paths
                    .iter()
                    .map(|(source, target)| (source.clone(), target.clone()))
                    .collect::<Vec<_>>();
                let state_warning =
                    sessions::relocate_state(&mut state, &path_updates, target_project).err();
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
                    let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                        generation: self.catalog_generation,
                        message,
                    });
                    // A backend may have completed part of a move before reporting an error.
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
