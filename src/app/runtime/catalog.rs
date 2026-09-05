use super::*;
use crate::sessions::activity::ActivityBuilder;

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
                let result = discover_catalog(locator_root.as_deref(), "").map(|mut discovery| {
                    recover_worker_execution(&mut discovery.sessions, &worker_families, |path| {
                        agents::load_external_history(path).transpose()
                    });
                    discovery
                });
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
                add_limited_activity_fallbacks(&mut activities, &all_sessions);
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
    fn worker_families_join_foreign_locators_by_harness_and_project() {
        let mut parent = summary(
            Path::new("/sessions/parent.jsonl"),
            SystemTime::now(),
            false,
        );
        parent.id = "parent-id".into();
        parent.harness = "pi".into();
        let mut child = summary(Path::new("/locators/child"), SystemTime::now(), false);
        child.id = "child-id".into();
        child.harness = "opencode2".into();
        let mut unrelated = child.clone();
        unrelated.harness = "codex-cli".into();
        let link = crate::agents::WorkerFamilyLink {
            project: PathBuf::from("/project"),
            child_backend: "opencode2".into(),
            child_session: "child-id".into(),
            parent_backend: "pi".into(),
            parent_session: "/sessions/parent.jsonl".into(),
            execution: None,
        };
        let mut sessions = vec![parent, child, unrelated];
        apply_worker_families(&mut sessions, &[link]);
        assert_eq!(sessions[1].parent_session.as_deref(), Some("parent-id"));
        assert!(sessions[2].parent_session.is_none());
    }

    #[test]
    fn legacy_worker_execution_is_recovered_only_for_the_matching_child() {
        let mut child = summary(Path::new("/locators/child"), SystemTime::now(), false);
        child.harness = "opencode2".into();
        let link = agents::WorkerFamilyLink {
            project: child.project.clone(),
            child_backend: child.harness.clone(),
            child_session: child.id.clone(),
            parent_backend: "pi".into(),
            parent_session: "/sessions/parent.jsonl".into(),
            execution: None,
        };
        let mut other = child.clone();
        other.harness = "codex-cli".into();
        let mut sessions = vec![other, child];
        let mut calls = 0;
        recover_worker_execution(&mut sessions, std::slice::from_ref(&link), |_| {
            calls += 1;
            Ok(Some(agents::DiscoveredHistory {
                messages: vec![],
                model: Some(("opencode-go".into(), "glm-5.3-flash".into())),
                thinking_level: Some("high".into()),
            }))
        });
        assert_eq!(calls, 1);
        assert!(sessions[0].model.is_none());
        assert_eq!(
            sessions[1].model,
            Some(("opencode-go".into(), "glm-5.3-flash".into()))
        );
        assert_eq!(sessions[1].thinking_level.as_deref(), Some("high"));
        let saved = agents::WorkerFamilyLink {
            execution: Some(agents::WorkerExecution {
                harness: "opencode2".into(),
                provider: "opencode-go".into(),
                model: "glm-5.3-flash".into(),
                effort: Some("high".into()),
            }),
            ..link
        };
        sessions[1].model = None;
        recover_worker_execution(&mut sessions, &[saved], |_| {
            panic!("saved identity must not reload history")
        });
    }

    #[test]
    fn every_external_write_refreshes_the_catalog_and_activity_deadline() {
        let path = PathBuf::from("/sessions/external.jsonl");
        let start = Instant::now();
        let mut activity = ExternalActivityTracker::default();

        assert!(activity.observe_files(
            &HashSet::new(),
            std::slice::from_ref(&path),
            start,
            crate::sessions::normalize_session_path,
        ));
        assert!(activity.observe_files(
            &HashSet::new(),
            std::slice::from_ref(&path),
            start + Duration::from_secs(1),
            crate::sessions::normalize_session_path,
        ));
        assert!(!activity.take_expired(start + RUNNING_ACTIVITY_TIMEOUT));
        assert!(activity.take_expired(start + Duration::from_secs(1) + RUNNING_ACTIVITY_TIMEOUT));

        let mut owned = ExternalActivityTracker::default();
        let owned_paths = HashSet::from([path.clone()]);
        assert!(!owned.observe_files(
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

    #[test]
    fn cached_sessions_get_truthful_limited_activity_fallbacks() {
        let session = summary(
            Path::new("/sessions/external.jsonl"),
            SystemTime::now(),
            false,
        );
        let mut activities = HashMap::new();

        add_limited_activity_fallbacks(&mut activities, &[session]);

        let activity = activities.get("external").expect("fallback activity");
        assert!(activity.limited);
        assert_eq!(
            activity.lifecycle,
            crate::agent_activity::AgentLifecycle::Unknown
        );
        assert_eq!(activity.role, "External");
    }

    #[test]
    fn parsed_activity_wins_over_a_limited_fallback() {
        let session = summary(
            Path::new("/sessions/external.jsonl"),
            SystemTime::now(),
            true,
        );
        let parsed = ActivityBuilder::default().finish(
            session.id.clone(),
            session.path.clone(),
            &session.title,
            "Working now",
            session.usage,
            session.modified,
            session.modified,
            true,
            false,
        );
        let mut activities = HashMap::from([(session.id.clone(), parsed.clone())]);

        add_limited_activity_fallbacks(&mut activities, &[session]);

        assert_eq!(activities.get("external"), Some(&parsed));
    }
}

fn apply_worker_families(
    sessions: &mut [crate::sessions::SessionSummary],
    links: &[crate::agents::WorkerFamilyLink],
) {
    for link in links {
        // Locators are opaque: match either the discovered ID or path, scoped by
        // harness and project. No backend's native locator is passed to another.
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

// Legacy links have no execution metadata. Recover it off the UI thread through
// the owning backend, then persist it during discovery application for reuse.
fn recover_worker_execution(
    sessions: &mut [SessionSummary],
    links: &[agents::WorkerFamilyLink],
    mut load: impl FnMut(&std::path::Path) -> Result<Option<agents::DiscoveredHistory>, String>,
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
        match load(&session.path) {
            Ok(Some(history)) => {
                session.model = history.model;
                session.thinking_level = history.thinking_level;
            }
            Ok(None) => {}
            Err(error) => {
                zlog::warn!(
                    "Recover worker execution for {}: {error}",
                    session.path.display()
                );
            }
        }
    }
}
