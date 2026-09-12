use super::*;

impl RuntimeOwner {
    pub(super) fn publish_child_session_metadata(&self, event: &Value) {
        let Some(mut child) = event.get("child").cloned() else {
            return;
        };
        child["harness"] = json!(self.harness);
        child["project"] = json!(self.project);
        if let Ok(metadata) = serde_json::from_value::<agents::SessionMetadata>(child.clone())
            && !metadata.id.is_empty()
        {
            let _ = self
                .event_tx
                .send(RuntimeEvent::AgentActivityUpdated(native_child_activity(
                    &child, &metadata,
                )));
            let _ = self.event_tx.send(RuntimeEvent::SessionMetadata(metadata));
        }
    }

    pub(super) fn publish_session_metadata(&self) {
        let snapshot = self.active_snapshot();
        let (Some(path), Some(session)) = (&self.active_session, &snapshot.session) else {
            return;
        };
        let first_user_message = snapshot
            .conversation
            .items
            .iter()
            .find(|item| {
                item.kind == crate::app::views::transcript::conversation::TranscriptKind::User
            })
            .map(|item| item.text.clone());
        let _ = self
            .event_tx
            .send(RuntimeEvent::SessionMetadata(agents::SessionMetadata {
                harness: self.harness.clone(),
                id: session.session_id.clone(),
                path: path.clone(),
                project: self.project.clone(),
                title: session.session_name.clone(),
                first_user_message,
                parent_session: None,
                message_count: Some(session.message_count),
                model: session
                    .model
                    .as_ref()
                    .map(|model| (model.provider.clone(), model.id.clone())),
                thinking_level: session.thinking_level.clone(),
                service_tier: session.service_tier.clone(),
                usage: snapshot
                    .stats
                    .get("tokens")
                    .filter(|tokens| tokens.is_object())
                    .map(|tokens| {
                        let number =
                            |key| tokens.get(key).and_then(Value::as_u64).unwrap_or_default();
                        agents::DiscoveredUsage {
                            input: number("input"),
                            output: number("output"),
                            cache_read: number("cacheRead"),
                            cache_write: number("cacheWrite"),
                            total: number("totalTokens"),
                            cost_micros: snapshot
                                .stats
                                .get("totalCost")
                                .and_then(Value::as_f64)
                                .map(|cost| (cost * 1_000_000.0).round() as u64)
                                .unwrap_or_default(),
                        }
                    }),
                is_running: snapshot.conversation.running,
            }));
    }

    pub(super) fn update_session_metadata(&mut self, metadata: agents::SessionMetadata) {
        let result = self
            .state
            .as_mut()
            .ok_or_else(|| "Session database is unavailable".to_owned())
            .and_then(|state| state.update_session_metadata(&metadata));
        let event = match result {
            Ok(session) => RuntimeEvent::SessionUpdated(session),
            Err(message) => RuntimeEvent::SessionsFailed {
                generation: self.session_generation,
                message,
            },
        };
        let _ = self.event_tx.send(event);
    }

    pub(super) fn load_sessions(&mut self, query: String) {
        self.session_query = query;
        self.publish_cached_sessions();
    }

    // Refresh only Farcaster's stored catalog. Backend discovery belongs to import.
    pub(super) fn refresh_sessions(&mut self) {
        if !self.owns_session_catalog {
            let _ = self.event_tx.send(RuntimeEvent::RefreshCatalog);
            return;
        }
        self.session_generation = self.session_generation.saturating_add(1);
        self.publish_cached_sessions();
    }

    pub(super) fn schedule_session_refresh(&mut self) {
        self.session_refresh_due
            .get_or_insert_with(|| Instant::now() + COALESCED_SESSION_REFRESH_DELAY);
    }

    pub(super) fn poll_deferred_session_refresh(&mut self, now: Instant) {
        if self.session_refresh_due.is_none_or(|due| now < due) {
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
        let known = match self
            .state
            .as_ref()
            .ok_or_else(|| "Session database is unavailable".to_owned())
            .and_then(|state| crate::sessions::cached_sessions(state, ""))
        {
            Ok(sessions) => sessions,
            Err(message) => {
                let _ = self.event_tx.send(RuntimeEvent::ImportPreviewFailed {
                    generation,
                    harness,
                    message,
                });
                return;
            }
        }
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
        if let Err(message) = self
            .state
            .as_mut()
            .ok_or_else(|| "Session database is unavailable".to_owned())
            .and_then(|state| crate::sessions::index_sessions(state, &sessions, false))
        {
            let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                generation: self.session_generation,
                message,
            });
            return;
        }
        self.session_generation = self.session_generation.saturating_add(1);
        self.publish_cached_sessions();
    }

    fn publish_cached_sessions(&self) {
        let Some(state) = &self.state else {
            return;
        };
        let event = match crate::sessions::cached_sessions(state, "") {
            Ok(sessions) => self.catalog_event(sessions),
            Err(message) => RuntimeEvent::SessionsFailed {
                generation: self.session_generation,
                message,
            },
        };
        let _ = self.event_tx.send(event);
    }

    fn catalog_event(&self, all_sessions: Vec<SessionSummary>) -> RuntimeEvent {
        let worker_activities = worker_activities(
            &all_sessions,
            crate::app::mcp_server::worker_snapshots().unwrap_or_default(),
        );
        RuntimeEvent::Sessions {
            generation: self.session_generation,
            sessions: crate::sessions::filter_session_tree(
                all_sessions.clone(),
                &self.session_query,
            ),
            all_sessions,
            // Pool snapshots are current lifecycle data. The projection merges
            // them without clearing richer history-backed activities.
            activities: (!worker_activities.is_empty()).then_some((worker_activities, false)),
        }
    }
}

fn worker_activities(
    sessions: &[SessionSummary],
    snapshots: Vec<agents::WorkerSnapshot>,
) -> HashMap<String, AgentActivity> {
    snapshots
        .into_iter()
        .filter_map(|snapshot| {
            let session = session_for_worker_snapshot(sessions, &snapshot)?;
            Some((
                crate::agent_activity::agent_activity_key(&session.path),
                AgentActivity::from_worker_snapshot(session, &snapshot),
            ))
        })
        .collect()
}

pub(in crate::app) fn native_child_activity(
    child: &Value,
    metadata: &agents::SessionMetadata,
) -> AgentActivity {
    AgentActivity::from_native_child(
        metadata.id.clone(),
        metadata.path.clone(),
        metadata.title.as_deref().unwrap_or("Agent"),
        metadata.is_running,
        child.get("outcome").and_then(Value::as_str),
    )
}

fn session_for_worker_snapshot<'a>(
    sessions: &'a [SessionSummary],
    snapshot: &agents::WorkerSnapshot,
) -> Option<&'a SessionSummary> {
    let locator = snapshot.session_locator.as_deref()?;
    sessions.iter().find(|session| {
        session.parent_session.is_some()
            && session.harness == snapshot.backend
            && session.project == snapshot.project
            && (session.id == locator || session.path == std::path::Path::new(locator))
    })
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

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
