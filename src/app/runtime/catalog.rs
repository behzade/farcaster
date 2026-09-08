use super::*;
use crate::sessions::activity::ActivityBuilder;

#[cfg(test)]
use crate::sessions::RUNNING_ACTIVITY_TIMEOUT;

impl RuntimeOwner {
    pub(super) fn load_sessions(&mut self, query: String) {
        self.session_query = query;
        self.publish_cached_sessions();
        if self.session_query.is_empty() {
            self.refresh_sessions();
        }
    }

    pub(super) fn refresh_sessions(&mut self) {
        if !self.owns_session_catalog {
            let _ = self.event_tx.send(RuntimeEvent::RefreshCatalog);
            return;
        }
        if self.session_discovery_in_flight {
            self.session_refresh_pending = true;
            return;
        }
        if self.session_refresh_due.is_some() {
            return;
        }
        self.session_generation = self.session_generation.saturating_add(1);
        self.session_discovery_in_flight = true;
        let generation = self.session_generation;
        let sender = self.discovery_tx.clone();
        let locator_root = self.process_command.session_locator_root.clone();
        let worker_families = self
            .state
            .as_ref()
            .map(|state| state.load_worker_families())
            .transpose()
            .unwrap_or_else(|error| {
                zlog::warn!("Load worker families for history recovery: {error}");
                None
            })
            .unwrap_or_default();
        let wake = thread::current();
        if let Err(error) = thread::Builder::new()
            .name("farcaster-sessions".into())
            .spawn(move || {
                let mut discovery = agents::discover_sessions(locator_root.as_deref(), "");
                recover_worker_execution(
                    &mut discovery.sessions,
                    &worker_families,
                    agents::load_session_history,
                );
                let _ = sender.send(DiscoveryResult {
                    generation,
                    result: Ok(discovery),
                });
                wake.unpark();
            })
        {
            self.session_discovery_in_flight = false;
            let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                generation,
                message: format!("start session discovery: {error}"),
            });
        }
    }

    pub(super) fn apply_discovery(&mut self, result: DiscoveryResult) {
        self.session_discovery_in_flight = false;
        if result.generation != self.session_generation {
            self.session_refresh_pending = false;
            self.schedule_session_refresh();
            return;
        }
        let event = match result.result {
            Ok(discovery) => {
                let mut discovered = discovery.sessions;
                if let Some(state) = &self.state {
                    match state.load_worker_families() {
                        Ok(mut links) => {
                            for link in &mut links {
                                if link.execution.is_none()
                                    && let Some(session) = discovered
                                        .iter()
                                        .find(|session| worker_child_matches(session, link))
                                    && let Some((provider, model)) = &session.model
                                {
                                    link.execution = Some(agents::WorkerExecution {
                                        harness: session.harness.clone(),
                                        provider: provider.clone(),
                                        model: model.clone(),
                                        effort: session.thinking_level.clone(),
                                    });
                                    if let Err(error) = state.save_worker_family(link) {
                                        zlog::warn!("Save recovered worker execution: {error}");
                                    }
                                }
                            }
                            apply_worker_families(&mut discovered, &links);
                        }
                        Err(error) => {
                            zlog::warn!("Load worker families: {error}");
                        }
                    }
                }
                let mut activities = discovery.activities;
                let running = discovered
                    .iter()
                    .filter(|session| session.is_running)
                    .map(|session| (session.harness.clone(), session.path.clone()))
                    .collect::<HashSet<_>>();
                let mut all_sessions = if let Some(state) = self.state.as_mut() {
                    match crate::sessions::index_sessions(state, &discovered, discovery.exhaustive)
                        .and_then(|()| crate::sessions::cached_sessions(state, ""))
                    {
                        Ok(sessions) => sessions,
                        Err(message) => {
                            let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                                generation: result.generation,
                                message,
                            });
                            discovered
                        }
                    }
                } else {
                    discovered
                };
                for session in &mut all_sessions {
                    session.is_running =
                        running.contains(&(session.harness.clone(), session.path.clone()));
                }
                add_limited_activity_fallbacks(&mut activities, &all_sessions);
                self.catalog_event(all_sessions, Some((activities, discovery.exhaustive)))
            }
            Err(message) => RuntimeEvent::SessionsFailed {
                generation: result.generation,
                message,
            },
        };
        let _ = self.event_tx.send(event);
        if std::mem::take(&mut self.session_refresh_pending) {
            self.session_refresh_due = Some(Instant::now() + COALESCED_SESSION_REFRESH_DELAY);
        }
    }

    pub(super) fn schedule_session_refresh(&mut self) {
        if self.session_discovery_in_flight {
            self.session_refresh_pending = true;
        } else {
            self.session_refresh_due
                .get_or_insert_with(|| Instant::now() + COALESCED_SESSION_REFRESH_DELAY);
        }
    }

    pub(super) fn poll_deferred_session_refresh(&mut self, now: Instant) {
        if self.session_discovery_in_flight || self.session_refresh_due.is_none_or(|due| now < due)
        {
            return;
        }
        self.session_refresh_due = None;
        self.refresh_sessions();
    }

    pub(super) fn preview_import(&mut self, harness: String, generation: u64) {
        if !self.owns_session_catalog {
            let _ = self.event_tx.send(RuntimeEvent::RefreshCatalog);
            return;
        }
        let known = self
            .state
            .as_ref()
            .and_then(|state| crate::sessions::cached_sessions(state, "").ok())
            .unwrap_or_default()
            .into_iter()
            .map(|session| crate::sessions::normalize_session_path(&session.path))
            .collect::<HashSet<_>>();
        let locator_root = self.process_command.session_locator_root.clone();
        let sender = self.event_tx.clone();
        let failed_harness = harness.clone();
        if let Err(error) = thread::Builder::new()
            .name("farcaster-import".into())
            .spawn(move || {
                let result = agents::discover_sessions_for(&harness, locator_root.as_deref(), "")
                    .map(|sessions| unknown_import_candidates(sessions, &known));
                let event = match result {
                    Ok(sessions) => RuntimeEvent::ImportPreview {
                        generation,
                        harness,
                        sessions,
                    },
                    Err(message) => RuntimeEvent::ImportPreviewFailed {
                        generation,
                        harness,
                        message,
                    },
                };
                let _ = sender.send(event);
            })
        {
            let _ = self.event_tx.send(RuntimeEvent::ImportPreviewFailed {
                generation,
                harness: failed_harness,
                message: format!("start import preview: {error}"),
            });
        }
    }

    pub(super) fn commit_import(&mut self, sessions: Vec<SessionSummary>) {
        if !self.owns_session_catalog {
            let _ = self.event_tx.send(RuntimeEvent::RefreshCatalog);
            return;
        }
        if sessions.is_empty() {
            return;
        }
        if let Some(state) = self.state.as_mut() {
            if let Err(message) = crate::sessions::index_sessions(state, &sessions, false) {
                let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                    generation: self.session_generation,
                    message,
                });
                return;
            }
        }
        self.session_generation = self.session_generation.saturating_add(1);
        self.publish_cached_sessions();
    }

    fn publish_cached_sessions(&self) {
        let Some(state) = &self.state else {
            return;
        };
        let event = match crate::sessions::cached_sessions(state, "") {
            Ok(sessions) => self.catalog_event(sessions, None),
            Err(message) => RuntimeEvent::SessionsFailed {
                generation: self.session_generation,
                message,
            },
        };
        let _ = self.event_tx.send(event);
    }

    fn catalog_event(
        &self,
        all_sessions: Vec<SessionSummary>,
        activities: Option<(HashMap<String, AgentActivity>, bool)>,
    ) -> RuntimeEvent {
        RuntimeEvent::Sessions {
            generation: self.session_generation,
            sessions: crate::sessions::filter_session_tree(
                all_sessions.clone(),
                &self.session_query,
            ),
            all_sessions,
            activities,
        }
    }
}

fn unknown_import_candidates(
    discovered: Vec<SessionSummary>,
    known_paths: &HashSet<std::path::PathBuf>,
) -> Vec<SessionSummary> {
    discovered
        .into_iter()
        .filter(|session| {
            !known_paths.contains(&crate::sessions::normalize_session_path(&session.path))
        })
        .collect()
}

fn add_limited_activity_fallbacks(
    activities: &mut HashMap<String, AgentActivity>,
    sessions: &[SessionSummary],
) {
    for session in sessions {
        activities.entry(session.id.clone()).or_insert_with(|| {
            ActivityBuilder::default().finish(
                session.id.clone(),
                session.path.clone(),
                &session.title,
                &session.first_user_message,
                session.usage,
                session.modified,
                session.modified,
                session.is_running,
                true,
            )
        });
    }
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;

fn apply_worker_families(
    sessions: &mut [crate::sessions::SessionSummary],
    links: &[crate::agents::WorkerFamilyLink],
) {
    for link in links {
        let matches = |session: &crate::sessions::SessionSummary, backend: &str, locator: &str| {
            session.harness == backend
                && session.project == link.project
                && (session.id == locator || session.path == std::path::Path::new(locator))
        };
        let parent = sessions
            .iter()
            .find(|session| matches(session, &link.parent_backend, &link.parent_session))
            .map(|session| session.id.clone());
        if let Some(parent) = parent {
            if let Some(child) = sessions
                .iter_mut()
                .find(|session| matches(session, &link.child_backend, &link.child_session))
            {
                child.parent_session = Some(parent);
            }
        }
    }
}

fn worker_child_matches(session: &SessionSummary, link: &agents::WorkerFamilyLink) -> bool {
    session.project == link.project
        && session.harness == link.child_backend
        && (session.id == link.child_session
            || session.path == std::path::Path::new(&link.child_session))
}

fn recover_worker_execution(
    sessions: &mut [SessionSummary],
    links: &[agents::WorkerFamilyLink],
    mut load: impl FnMut(&str, &std::path::Path) -> Result<LoadedHistory, String>,
) {
    for link in links {
        if link.execution.is_some() {
            continue;
        }
        let Some(session) = sessions
            .iter_mut()
            .find(|session| worker_child_matches(session, link))
        else {
            continue;
        };
        if session.model.is_some() {
            continue;
        }
        match load(&session.harness, &session.path) {
            Ok(history) => {
                session.model = history.model;
                session.thinking_level = history.thinking_level;
            }
            Err(error) => {
                zlog::warn!(
                    "Recover worker execution for {}: {error}",
                    session.path.display()
                );
            }
        }
    }
}
