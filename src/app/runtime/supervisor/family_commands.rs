use super::*;

impl Supervisor {
    pub(super) fn handle_session_family_command(&mut self, command: &RuntimeCommand) -> bool {
        if let RuntimeCommand::StopSessionFamily { path } = &command {
            if let Some(family) = session_family_for_path(&self.catalog_sessions, path) {
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
                        actor.join();
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
                        snapshot.conversation.running
                            || snapshot.conversation.compacting
                            || self.needs_input.contains(key)
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
                        actor.join();
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
                let leftovers = delete_session_files(&paths)?;
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
                        snapshot.conversation.running
                            || snapshot.conversation.compacting
                            || self.needs_input.contains(key)
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
                        actor.join();
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
                let members = family
                    .iter()
                    .map(|session| TransferMember {
                        path: session.path.clone(),
                        id: session.id.clone(),
                        parent_id: session.parent_session.clone(),
                    })
                    .collect::<Vec<_>>();
                let session_root = configured_session_root()?;
                let destination =
                    sessions::destination_directory(&session_root, target_project, &root.path);
                let moved =
                    sessions::move_family(&members, &root.id, target_project, &destination)?;
                let path_updates = moved
                    .paths
                    .iter()
                    .map(|(source, target)| (source.clone(), target.clone()))
                    .collect::<Vec<_>>();
                let state_warning =
                    sessions::relocate_state(&mut state, &path_updates, target_project).err();
                Ok((moved, state_warning))
            })();
            match result {
                Ok((moved, state_warning)) => {
                    let _ = self.event_tx.send(RuntimeEvent::SessionMoved {
                        target_root: moved.root,
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
                }
            }
            return true;
        }
        false
    }
}
