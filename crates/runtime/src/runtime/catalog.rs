use super::*;
use crate::agents::Backend;

impl RuntimeOwner {
    pub(super) fn publish_child_session_metadata(&self, event: &Value) {
        let Some(mut child) = event.get("child").cloned() else {
            return;
        };
        child["harness"] = json!(self.harness);
        child["project"] = json!(self.project);
        child["profile_id"] = json!(self.process_command.profile_id);
        if let Ok(metadata) = serde_json::from_value::<agents::SessionMetadata>(child.clone())
            && !metadata.id.is_empty()
        {
            let activity = native_child_activity(&child, &metadata);
            let path = crate::sessions::normalize_session_path(&activity.session_path);
            let profile_id = metadata
                .profile_id
                .clone()
                .or_else(|| crate::sessions::profile_id_from_locator(&path));
            let _ = self.event_tx.send(RuntimeEvent::AgentActivityUpdated {
                key: crate::agent_activity::WorkerActivityKey {
                    path,
                    native: crate::sessions::NativeSessionIdentity {
                        project: crate::sessions::normalize_session_path(&metadata.project),
                        harness: metadata.harness,
                        profile_id,
                        id: metadata.id.clone(),
                    },
                },
                activity,
            });
            let _ = self.event_tx.send(RuntimeEvent::SessionMetadata(metadata));
        }
    }

    pub(super) fn save_observed_facts(
        &self,
        path: &std::path::Path,
        draft: Option<&str>,
    ) -> Result<(), String> {
        let Some(harness) = self.harness else {
            return Ok(());
        };
        self.session_repository()?.record_session_facts(
            farcaster_storage::ObservedSession {
                harness,
                project: &self.project,
                path,
                profile_id: self.process_command.profile_id.as_deref(),
                draft,
            },
            &self.active_snapshot().session_facts(),
        )
    }

    pub(super) fn publish_session_metadata(&self) {
        if let Some(path) = &self.active_session
            && let Err(error) = self.save_observed_facts(path, None)
        {
            zlog::warn!("Save session facts: {error}");
        }
        self.send_session_metadata();
    }

    pub(super) fn send_session_metadata(&self) {
        let Some(harness) = self.harness else { return };
        let snapshot = self.active_snapshot();
        let (Some(path), Some(session)) = (&self.active_session, &snapshot.session) else {
            return;
        };
        let first_user_message = snapshot
            .conversation
            .items
            .iter()
            .find(|item| item.kind == crate::conversation::TranscriptKind::User)
            .map(|item| item.text.clone());
        let _ = self
            .event_tx
            .send(RuntimeEvent::SessionMetadata(agents::SessionMetadata {
                harness,
                profile_id: self.process_command.profile_id.clone(),
                id: session.session_id.clone(),
                path: path.clone(),
                project: self.project.clone(),
                title: session.session_name.clone(),
                first_user_message,
                parent_session: None,
                forked_from: self.forked_from.clone(),
                message_count: Some(session.message_count),
                model: session
                    .model
                    .as_ref()
                    .map(|model| (model.provider.clone(), model.id.clone())),
                thinking_level: session.thinking_level.clone(),
                service_tier: snapshot.observed_service_tier.clone(),
                access_mode: Some(self.process_command.access_mode),
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

    pub(super) fn update_session_metadata(
        &self,
        metadata: &agents::SessionMetadata,
    ) -> Result<(), String> {
        let session = self
            .session_repository()?
            .record_session_metadata(metadata)?;
        let _ = self.event_tx.send(RuntimeEvent::SessionUpdated(session));
        Ok(())
    }

    pub(super) fn set_catalog_archive(
        &self,
        request: ArchiveRequest,
        draft_saved: Option<async_channel::Receiver<Result<crate::sessions::AppSessionId, String>>>,
    ) {
        let rows = (|| {
            let target = if let Some(saved) = draft_saved {
                let id = saved
                    .recv_blocking()
                    .map_err(|_| "Session state writer stopped before saving".to_owned())??;
                if let ArchiveTarget::Draft(draft) = &request.target {
                    let _ = self.event_tx.send(RuntimeEvent::DraftIdentitySaved {
                        draft: draft.clone(),
                        app_session_id: id,
                    });
                }
                ArchiveTarget::App(id)
            } else {
                request.target.clone()
            };
            self.state
                .as_ref()
                .ok_or("Session database is unavailable")?
                .with(|store| match &target {
                    ArchiveTarget::Draft(_) => Err("The chat has no saved identity".into()),
                    ArchiveTarget::App(id) => Ok(store
                        .archive_app_session(*id, request.archived)?
                        .into_iter()
                        .collect()),
                    ArchiveTarget::Native(_) | ArchiveTarget::Path(_) => {
                        let (path, native) = match &target {
                            ArchiveTarget::Native(t) => (&t.path, Some(t)),
                            ArchiveTarget::Path(path) => (path, None),
                            _ => unreachable!(),
                        };
                        let rows =
                            store.archive_matching_sessions(path, native, request.archived)?;
                        if rows.is_empty() {
                            return Err(format!(
                                "The session is no longer available to archive: {}",
                                path.display()
                            ));
                        }
                        Ok(rows)
                    }
                })
        })();
        let _ = self
            .event_tx
            .send(RuntimeEvent::SessionArchiveResult { request, rows });
    }

    pub(super) fn load_sessions(&mut self, query: String) {
        self.session_query = query;
        self.publish_cached_sessions();
    }

    pub(super) fn refresh_worker_status(&mut self, id: &str) {
        // Read current status on the catalog actor so a pending signal cannot replay an old
        // status over a newer full catalog. Idle workers without output need stored outcomes.
        if let Ok(Some(snapshot)) = self.host.worker_status_snapshot(id)
            && (snapshot.status != agents::WorkerStatus::Idle || snapshot.output.is_some())
        {
            let _ = self.event_tx.send(RuntimeEvent::WorkerSnapshot(snapshot));
        } else {
            self.refresh_sessions();
        }
    }

    pub(super) fn refresh_sessions(&mut self) {
        if !self.owns_session_catalog {
            let _ = self.event_tx.send(RuntimeEvent::RefreshCatalog);
            return;
        }
        self.session_generation = self.session_generation.saturating_add(1);
        self.publish_cached_sessions();
    }

    pub(super) fn preview_import(
        &mut self,
        harness: Backend,
        profile_id: Option<String>,
        generation: u64,
    ) {
        if !self.owns_session_catalog {
            let _ = self.event_tx.send(RuntimeEvent::RefreshCatalog);
            return;
        }
        let known = match self
            .state
            .as_ref()
            .ok_or_else(|| "Session database is unavailable".to_owned())
            .and_then(|state| state.with(|store| crate::sessions::cached_sessions(store, "")))
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
        let mut command = self.process_command.clone();
        command.profile_id = profile_id;
        let sender = self.event_tx.clone();
        let failed_harness = harness;
        if let Err(error) = thread::Builder::new()
            .name("farcaster-import".into())
            .spawn(move || {
                let result = agents::discover_sessions_for_profile(&command, harness, "")
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
            .and_then(|state| {
                state.with(|store| crate::sessions::index_sessions(store, &sessions, false))
            })
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
        let event = match state.with(|store| {
            Ok((
                crate::sessions::cached_sessions(store, "")?,
                store.worker_outcomes()?,
            ))
        }) {
            Ok((sessions, outcomes)) => self.catalog_event(sessions, outcomes),
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
        outcomes: std::collections::BTreeMap<i64, Option<crate::agent_activity::AgentOutcome>>,
    ) -> RuntimeEvent {
        let mut worker_activities = worker_activities(
            &all_sessions,
            self.host.worker_snapshots().unwrap_or_default(),
        );
        for session in &all_sessions {
            let Some(outcome) = outcomes.get(&session.app_session_id) else {
                continue;
            };
            let key = session.activity_key();
            if worker_activities.get(&key).is_some_and(|activity| {
                activity.lifecycle != crate::agent_activity::AgentLifecycle::Unknown
            }) {
                continue;
            }
            worker_activities.insert(key, AgentActivity::from_worker_outcome(session, *outcome));
        }
        RuntimeEvent::Sessions {
            generation: self.session_generation,
            sessions: crate::sessions::filter_session_tree(
                all_sessions.clone(),
                &self.session_query,
            ),
            all_sessions,
            activities: (!worker_activities.is_empty()).then_some((worker_activities, false)),
        }
    }
}

pub(super) fn worker_activities(
    sessions: &[SessionSummary],
    snapshots: Vec<agents::WorkerSnapshot>,
) -> HashMap<crate::agent_activity::WorkerActivityKey, AgentActivity> {
    snapshots
        .into_iter()
        .filter_map(|snapshot| {
            let session = session_for_worker_snapshot(sessions, &snapshot)?;
            Some((
                session.activity_key(),
                AgentActivity::from_worker_status(
                    session,
                    snapshot.status,
                    snapshot.output.is_some(),
                ),
            ))
        })
        .collect()
}

#[cfg(any(test, feature = "test-support"))]
pub fn live_worker_activities(
    sessions: &[SessionSummary],
    snapshots: Vec<agents::WorkerSnapshot>,
) -> HashMap<crate::agent_activity::WorkerActivityKey, AgentActivity> {
    worker_activities(sessions, snapshots)
}

pub fn native_child_activity(child: &Value, metadata: &agents::SessionMetadata) -> AgentActivity {
    AgentActivity::from_native_child(
        metadata.id.clone(),
        metadata.path.clone(),
        metadata.title.as_deref().unwrap_or("Agent"),
        metadata.is_running,
        child.get("outcome").and_then(Value::as_str),
    )
}

pub(super) fn session_for_worker_snapshot<'a>(
    sessions: &'a [SessionSummary],
    snapshot: &agents::WorkerSnapshot,
) -> Option<&'a SessionSummary> {
    let key = snapshot.session_key.as_ref()?;
    let mut matches = sessions.iter().filter(|session| {
        (session.parent_session.is_some() || session.parent_app_session_id.is_some())
            && session.harness == snapshot.backend
            && session.project == snapshot.project
            && session.key() == *key
    });
    let session = matches.next()?;
    matches.next().is_none().then_some(session)
}

fn unknown_import_candidates(
    discovered: Vec<SessionSummary>,
    known_paths: &HashSet<std::path::PathBuf>,
) -> Vec<SessionSummary> {
    discovered
        .into_iter()
        .filter(|session| {
            session.parent_session.is_none()
                && !known_paths.contains(&crate::sessions::normalize_session_path(&session.path))
        })
        .collect()
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
