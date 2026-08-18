//! Session catalog loading, filtering, and transient activity publication.

use super::*;

impl RuntimeOwner {
    pub(super) fn load_sessions(&mut self, query: String) {
        self.session_query = query;
        if let Some(state) = &self.state {
            match state.cached_sessions("") {
                Ok(all_sessions) => {
                    let sessions = crate::sessions::filter_session_tree(
                        all_sessions.clone(),
                        &self.session_query,
                    );
                    let has_running_descendants = has_running_descendant(&all_sessions);
                    let _ = self.event_tx.send(RuntimeEvent::Sessions {
                        generation: self.session_generation,
                        sessions,
                        all_sessions,
                        activities: None,
                        has_running_descendants,
                    });
                }
                Err(error) => {
                    let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                        generation: self.session_generation,
                        message: error,
                    });
                }
            }
        }
        if self.session_query.is_empty() {
            self.refresh_sessions();
        }
    }

    pub(super) fn refresh_active_sessions(&mut self) {
        if !self.owns_session_catalog {
            return;
        }
        let paths = self
            .state
            .as_ref()
            .and_then(|state| state.cached_sessions("").ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|session| session.is_running)
            .map(|session| session.path)
            .collect::<Vec<_>>();
        if paths.is_empty() || self.session_discovery_in_flight {
            return;
        }
        self.session_generation = self.session_generation.saturating_add(1);
        self.session_discovery_in_flight = true;
        let generation = self.session_generation;
        let sender = self.discovery_tx.clone();
        if let Err(error) = thread::Builder::new()
            .name("pi-gpui-active-sessions".into())
            .spawn(move || {
                let _ = sender.send(DiscoveryResult {
                    generation,
                    result: discover_paths(&paths),
                });
            })
        {
            self.session_discovery_in_flight = false;
            let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                generation,
                message: format!("start active session refresh: {error}"),
            });
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
        if let Err(error) = thread::Builder::new()
            .name("pi-gpui-sessions".into())
            .spawn(move || {
                let _ = sender.send(DiscoveryResult {
                    generation,
                    result: discover(""),
                });
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
        if result.generation != self.session_generation {
            return;
        }
        self.session_discovery_in_flight = false;
        let event = match result.result {
            Ok(discovery) => {
                let discovered = discovery.sessions;
                let activities = discovery.activities;
                let (sessions, all_sessions) = if let Some(state) = self.state.as_mut() {
                    match state
                        .index_sessions(&discovered, discovery.exhaustive)
                        .and_then(|()| state.cached_sessions(""))
                    {
                        Ok(all_sessions) => {
                            let sessions = crate::sessions::filter_session_tree(
                                all_sessions.clone(),
                                &self.session_query,
                            );
                            (sessions, all_sessions)
                        }
                        Err(error) => {
                            let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                                generation: result.generation,
                                message: error,
                            });
                            let sessions = crate::sessions::filter_session_tree(
                                discovered.clone(),
                                &self.session_query,
                            );
                            (sessions, discovered)
                        }
                    }
                } else {
                    let sessions = crate::sessions::filter_session_tree(
                        discovered.clone(),
                        &self.session_query,
                    );
                    (sessions, discovered)
                };
                let has_running_descendants = has_running_descendant(&all_sessions);
                RuntimeEvent::Sessions {
                    generation: result.generation,
                    sessions,
                    all_sessions,
                    activities: Some((activities, discovery.exhaustive)),
                    has_running_descendants,
                }
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

    pub(super) fn poll_deferred_session_refresh(&mut self, now: Instant) {
        if self.session_discovery_in_flight || self.session_refresh_due.is_none_or(|due| now < due)
        {
            return;
        }
        self.session_refresh_due = None;
        self.refresh_sessions();
    }
}
