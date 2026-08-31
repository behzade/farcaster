use super::*;

#[cfg(test)]
use crate::sessions::RUNNING_ACTIVITY_TIMEOUT;

fn discover_catalog(
    locator_root: Option<&std::path::Path>,
    query: &str,
) -> Result<SessionDiscovery, String> {
    let mut discovery = sessions::discover(query)?;
    let (external, external_exhaustive) = agents::discover_external_sessions(locator_root, query);
    discovery
        .sessions
        .extend(external.into_iter().map(import_agent_session));
    discovery.exhaustive &= external_exhaustive;
    discovery
        .sessions
        .sort_by_key(|session| std::cmp::Reverse(session.modified));
    Ok(discovery)
}

impl RuntimeOwner {
    pub(super) fn load_sessions(&mut self, query: String) {
        self.session_query = query;
        if let Some(state) = &self.state {
            match crate::sessions::cached_sessions(state, "") {
                Ok(mut all_sessions) => {
                    if self.session_generation == 0 {
                        for session in &mut all_sessions {
                            session.is_running = false;
                        }
                    }
                    let sessions = crate::sessions::filter_session_tree(
                        all_sessions.clone(),
                        &self.session_query,
                    );
                    let _ = self.event_tx.send(RuntimeEvent::Sessions {
                        generation: self.session_generation,
                        sessions,
                        all_sessions,
                        activities: None,
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
        let wake = thread::current();
        if let Err(error) = thread::Builder::new()
            .name("farcaster-sessions".into())
            .spawn(move || {
                let result = discover_catalog(locator_root.as_deref(), "");
                let _ = sender.send(DiscoveryResult { generation, result });
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
        if result.generation != self.session_generation {
            return;
        }
        self.session_discovery_in_flight = false;
        let event = match result.result {
            Ok(discovery) => {
                let discovered = discovery.sessions;
                let activities = discovery.activities;
                let (sessions, all_sessions) = if let Some(state) = self.state.as_mut() {
                    match crate::sessions::index_sessions(state, &discovered, discovery.exhaustive)
                        .and_then(|()| crate::sessions::cached_sessions(state, ""))
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
                RuntimeEvent::Sessions {
                    generation: result.generation,
                    sessions,
                    all_sessions,
                    activities: Some((activities, discovery.exhaustive)),
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
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn summary(path: &Path, modified: SystemTime, is_running: bool) -> SessionSummary {
        SessionSummary::from_cached(
            "external".into(),
            path.to_path_buf(),
            PathBuf::from("/project"),
            "External".into(),
            String::new(),
            String::new(),
            None,
            modified,
            0,
            crate::sessions::UsageSummary::default(),
            false,
            is_running,
            String::new(),
        )
    }

    #[test]
    fn heartbeats_refresh_only_at_activity_boundaries() {
        let path = PathBuf::from("/sessions/external.jsonl");
        let dormant = summary(&path, SystemTime::now(), false);
        let start = Instant::now();
        let mut activity = ExternalActivityTracker::default();

        assert!(activity.observe_files(
            std::slice::from_ref(&dormant),
            &HashSet::new(),
            std::slice::from_ref(&path),
            start,
            crate::sessions::normalize_session_path,
        ));
        assert!(!activity.observe_files(
            std::slice::from_ref(&dormant),
            &HashSet::new(),
            std::slice::from_ref(&path),
            start + Duration::from_secs(1),
            crate::sessions::normalize_session_path,
        ));
        assert!(!activity.take_expired(start + RUNNING_ACTIVITY_TIMEOUT));
        assert!(activity.take_expired(start + Duration::from_secs(1) + RUNNING_ACTIVITY_TIMEOUT));

        let mut running = dormant.clone();
        running.is_running = true;
        let mut active = ExternalActivityTracker::default();
        assert!(!active.observe_files(
            &[running],
            &HashSet::new(),
            std::slice::from_ref(&path),
            start,
            crate::sessions::normalize_session_path,
        ));
        assert!(active.take_expired(start + RUNNING_ACTIVITY_TIMEOUT));

        let mut owned = ExternalActivityTracker::default();
        let owned_paths = HashSet::from([path.clone()]);
        assert!(!owned.observe_files(
            &[dormant],
            &owned_paths,
            &[path],
            start,
            crate::sessions::normalize_session_path,
        ));
        assert!(!owned.take_expired(start + RUNNING_ACTIVITY_TIMEOUT));
    }

    #[test]
    fn catalog_sync_seeds_the_remaining_activity_deadline() {
        let wall_now = SystemTime::now();
        let now = Instant::now();
        let session = summary(
            Path::new("/sessions/external.jsonl"),
            wall_now - Duration::from_secs(5),
            true,
        );
        let mut activity = ExternalActivityTracker::default();

        activity.sync_catalog(&[session], true, &HashSet::new(), now, wall_now);

        assert!(!activity.take_expired(now + RUNNING_ACTIVITY_TIMEOUT - Duration::from_secs(6)));
        assert!(activity.take_expired(now + RUNNING_ACTIVITY_TIMEOUT - Duration::from_secs(5)));
    }
}
