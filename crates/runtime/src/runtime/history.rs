use super::*;
use crate::agents::Backend;
use farcaster_storage::{SessionRepository, StoredTranscript, TranscriptSession};

#[path = "stored_history.rs"]
mod stored_history;

fn restored_question_request(question: crate::sessions::RestoredQuestion) -> ExtensionUiRequest {
    if question.options.is_empty() {
        ExtensionUiRequest::Input {
            id: question.id,
            title: question.title,
            placeholder: None,
            timeout: None,
        }
    } else {
        ExtensionUiRequest::Select {
            id: question.id,
            title: question.title,
            options: question.options,
            timeout: None,
        }
    }
}

impl RuntimeOwner {
    #[cfg(test)]
    pub(super) fn select_history(&mut self, path: PathBuf, project: PathBuf) {
        self.select_document(path, project, None);
    }

    pub(super) fn select_document(
        &mut self,
        path: PathBuf,
        project: PathBuf,
        display_stamp: Option<farcaster_storage::TranscriptStamp>,
    ) {
        let _timing = self.host.timer(RuntimeMetric::SelectDocument);
        self.history_generation = self.history_generation.saturating_add(1);
        self.pending_document_refresh = None;
        if self.covers_resident_session(&path) && !self.snapshot.needs_saved_revision(display_stamp)
        {
            if !self.showing_live_session(&path) {
                self.restore_live_session(project);
            }
            return;
        }
        if self.snapshot.selected_session.as_deref() == Some(path.as_path())
            && self.snapshot.document_ready
            && !self.snapshot.needs_saved_revision(display_stamp)
        {
            return;
        }
        if self.bind_history_selection(path.clone(), project.clone()) {
            self.refresh_history(
                path,
                project,
                if display_stamp.is_some() {
                    HistoryLoadKind::DisplayedSelection
                } else {
                    HistoryLoadKind::Selection
                },
            );
        }
    }

    fn covers_resident_session(&self, path: &std::path::Path) -> bool {
        self.active_session.as_deref() == Some(path)
            && (self.process.is_some()
                || self.parked_snapshot.is_some()
                || self.snapshot.document_ready)
    }

    fn showing_live_session(&self, path: &std::path::Path) -> bool {
        self.parked_snapshot.is_none()
            && !self.snapshot.history_preview
            && self.snapshot.selected_session.as_deref() == Some(path)
    }

    fn restore_live_session(&mut self, project: PathBuf) {
        if let Some(snapshot) = self.parked_snapshot.take() {
            self.snapshot = snapshot;
        }
        self.project = project.clone();
        self.snapshot.project = project;
        self.transcript_changed_from = Some(0);
        let _ = self.event_tx.send(RuntimeEvent::HistoryReset {
            generation: self.process_generation,
        });
        self.publish();
    }

    pub(super) fn bind_external_session_identity(&mut self, path: &std::path::Path) {
        if let Some((harness, session_id)) = agents::external_session_identity(path)
            && self.harness.is_none_or(|known| known == harness)
            && (self.session_id.is_none()
                || self.session_id.as_deref() == Some(path.to_string_lossy().as_ref()))
        {
            self.harness = Some(harness);
            self.session_id = Some(session_id);
        }
    }

    fn bind_history_selection(&mut self, path: PathBuf, project: PathBuf) -> bool {
        let profile_id = match configuration_for_target(
            &self.process_command,
            self.state.as_ref(),
            LaunchTarget::Session(&path, self.harness),
        ) {
            Ok(config) => config.profile_id,
            Err(error) => {
                self.fail(error);
                return false;
            }
        };
        if self.process.is_none() && self.snapshot.selected_session.as_ref() != Some(&path) {
            self.pending_session_controls = PendingSessionControls::default();
        }
        self.bind_external_session_identity(&path);
        let facts = self.session_facts(&path);
        if self.parked_snapshot.is_none()
            && self.snapshot.selected_session.as_deref() != Some(path.as_path())
            && (self.process.is_some()
                || self.active_session.is_some()
                || !self.snapshot.conversation.items.is_empty())
        {
            self.parked_snapshot = Some(self.snapshot.clone());
        }
        self.project = project.clone();
        self.snapshot.project = project;
        self.snapshot.selected_session = Some(path);
        self.snapshot.profile_id = profile_id;
        self.snapshot.status = "Loading history".into();
        self.snapshot.document_ready = false;
        self.snapshot.saved_history = None;
        self.snapshot.transcript_stamp = None;
        self.snapshot.history_tail = None;
        self.snapshot.session = None;
        self.snapshot.conversation = Default::default();
        self.snapshot.restore_session_facts(facts);
        self.transcript_changed_from = Some(0);
        self.publish();
        true
    }

    pub(super) fn refresh_session_document(&mut self, path: PathBuf, project: PathBuf) {
        if self.active_session.as_deref() == Some(path.as_path()) && self.process.is_some() {
            return;
        }
        if self.history_selection_generation.is_some() {
            self.pending_document_refresh = Some((path, project));
            return;
        }
        if self.document_refresh_generation.is_some() {
            self.pending_document_refresh = Some((path, project));
            return;
        }
        self.refresh_history(path, project, HistoryLoadKind::DocumentRefresh);
    }

    pub(super) fn refresh_history(
        &mut self,
        path: PathBuf,
        project: PathBuf,
        kind: HistoryLoadKind,
    ) {
        self.history_generation = self.history_generation.saturating_add(1);
        let generation = self.history_generation;
        *self.history_load_generation_mut(kind) = Some(generation);
        let sender = self.history_tx.clone();
        let wake = thread::current();
        let failed_path = path.clone();
        let failed_project = project.clone();
        let harness = self.harness;
        let config = configuration_for_target(
            &self.process_command,
            self.state.as_ref(),
            LaunchTarget::Session(&path, self.harness),
        );
        let host = self.host.clone();
        let repository = self.state.clone();
        if let Err(error) = thread::Builder::new()
            .name("farcaster-history".into())
            .spawn(move || {
                let mut operation = host.timer(RuntimeMetric::LoadHistory);
                let request = stored_history::HistoryRequest {
                    generation,
                    path: &path,
                    project: &project,
                    kind,
                    sender: &sender,
                    wake: &wake,
                };
                let result = harness
                    .ok_or_else(|| "Choose a backend before loading history.".to_owned())
                    .and_then(|harness| request.read(&config?, harness, repository.as_ref()));
                match result {
                    Ok(None) => {} // The local read already published the complete document.
                    Ok(Some((history, stamp))) => {
                        operation.set_work(history.messages.len());
                        request.finish(Ok(history), stamp);
                    }
                    Err(error) => request.finish(Err(error), None),
                }
            })
        {
            self.apply_history(HistoryResult {
                transcript_stamp: None,
                generation,
                path: failed_path,
                project: failed_project,
                kind,
                result: Err(format!("start session history load: {error}")),
            });
        }
    }

    pub(super) fn history_load_generation_mut(
        &mut self,
        kind: HistoryLoadKind,
    ) -> &mut Option<u64> {
        match kind {
            HistoryLoadKind::Selection | HistoryLoadKind::DisplayedSelection => {
                &mut self.history_selection_generation
            }
            HistoryLoadKind::DocumentRefresh => &mut self.document_refresh_generation,
        }
    }

    pub(super) fn invalidate_history_loads(&mut self) {
        self.history_generation = self.history_generation.saturating_add(1);
        self.history_selection_generation = None;
        self.document_refresh_generation = None;
        self.pending_document_refresh = None;
    }

    pub(super) fn stage_draft(
        &mut self,
        target: LaunchTarget<'_>,
        harness: Option<Backend>,
        project: PathBuf,
    ) {
        let config =
            match configuration_for_target(&self.process_command, self.state.as_ref(), target) {
                Ok(config) => config,
                Err(error) => {
                    conversation_mut(&mut self.snapshot)
                        .push_local_error("Load harness profile", error);
                    self.publish();
                    return;
                }
            };
        let unchanged = self.process.is_none()
            && self.parked_snapshot.is_none()
            && !self.snapshot.history_preview
            && self.harness == harness
            && self.process_command.profile_id == config.profile_id
            && self.project == project;
        if unchanged {
            self.snapshot.profile_id = config.profile_id;
            self.publish();
            return;
        }

        self.reset_process_runtime();
        self.harness = harness;
        self.process_command = config;
        self.project = project.clone();
        self.session_id = None;
        self.pending_session_controls = PendingSessionControls::default();
        reset_snapshot_for_process(&mut self.snapshot, project, None, "Ready".into());
        self.snapshot.profile_id = self.process_command.profile_id.clone();
        self.publish();
    }

    pub(super) fn apply_history_update(&mut self, update: HistoryUpdate) {
        match update {
            HistoryUpdate::Finished(result) => self.apply_history(result),
            HistoryUpdate::Saved {
                result,
                refreshing,
                complete,
            } => {
                let saved = if complete {
                    SavedHistory::Complete
                } else if refreshing {
                    SavedHistory::RefreshingPartial
                } else {
                    SavedHistory::Partial
                };
                self.project_history(result, Some(saved), !refreshing);
            }
            HistoryUpdate::Tail { generation, page } => {
                if self.history_selection_generation != Some(generation)
                    || self.history_generation != generation
                    || self.snapshot.document_ready
                    || self.snapshot.history_tail.is_some()
                    || self.snapshot.status != "Loading history"
                    || page.content.is_empty()
                {
                    return;
                }
                let Some(path) = self.snapshot.selected_session.as_deref() else {
                    return;
                };
                // Full-history reconciliation can append an unresolved local prompt or
                // replace a repeated prompt's display text. Wait when either is possible.
                if let (Some(state), Some(harness)) = (&self.state, self.harness)
                    && !state
                        .can_display_transcript_tail(TranscriptSession {
                            harness,
                            path,
                            project: &self.snapshot.project,
                            profile_id: self.snapshot.profile_id.as_deref(),
                        })
                        .unwrap_or(false)
                {
                    return;
                }
                let mut tail = ConversationState::default();
                tail.replace_history(&page.content);
                self.snapshot.history_tail = Some(Arc::new(agents::HistoryPage {
                    content: tail,
                    coverage: page.coverage,
                }));
                self.publish();
            }
        }
    }

    pub(super) fn apply_history(&mut self, result: HistoryResult) {
        self.project_history(result, None, true);
    }

    fn project_history(
        &mut self,
        result: HistoryResult,
        saved: Option<SavedHistory>,
        finished: bool,
    ) {
        let active_generation = self.history_load_generation_mut(result.kind);
        if finished && *active_generation == Some(result.generation) {
            *active_generation = None;
        }
        if result.generation != self.history_generation {
            self.start_pending_document_refresh();
            return;
        }
        if (result.kind != HistoryLoadKind::DocumentRefresh
            && self.snapshot.saved_history.is_none()
            && self.covers_resident_session(&result.path))
            || (self.active_session.as_ref() == Some(&result.path) && self.process.is_some())
        {
            self.start_pending_document_refresh();
            return;
        }
        let config = configuration_for_target(
            &self.process_command,
            self.state.as_ref(),
            LaunchTarget::Session(&result.path, self.harness),
        );
        let same_document = self.snapshot.document_ready
            && self.snapshot.selected_session.as_ref() == Some(&result.path);
        if same_document
            && self.snapshot.saved_history.is_some()
            && let Some(error) = result.result.as_ref().err().or(config.as_ref().err())
        {
            zlog::warn!("Could not refresh the saved transcript: {error}");
            if self.snapshot.saved_history == Some(SavedHistory::RefreshingPartial) {
                self.snapshot.saved_history = Some(SavedHistory::Partial);
                self.publish();
            }
            self.start_pending_document_refresh();
            return;
        }
        let profile_id = match config {
            Ok(config) => config.profile_id,
            Err(error) => {
                self.fail(error);
                self.start_pending_document_refresh();
                return;
            }
        };
        self.bind_external_session_identity(&result.path);
        let refreshing_visible_history = same_document
            && (self.snapshot.saved_history.is_some()
                || result.kind == HistoryLoadKind::DocumentRefresh);
        self.snapshot.history_tail = None;
        let mut history = match result.result {
            Ok(history) => history,
            Err(error) => {
                self.project = result.project.clone();
                self.snapshot.project = result.project;
                self.snapshot.selected_session = Some(result.path);
                self.snapshot.profile_id = profile_id;
                self.snapshot.status = "Could not load history".into();
                conversation_mut(&mut self.snapshot).push_local_error("History unavailable", error);
                self.publish();
                self.start_pending_document_refresh();
                return;
            }
        };
        let projection = self.host.timer(RuntimeMetric::ProjectHistory);
        let transcript_stamp = if let (Some(state), Some(harness)) = (&self.state, self.harness) {
            annotate_session_prompts(
                state,
                TranscriptSession {
                    harness,
                    path: &result.path,
                    project: &result.project,
                    profile_id: profile_id.as_deref(),
                },
                history.prompt_deliveries.as_ref(),
                &mut history.messages,
            )
            .filter(|stamp| Some(*stamp) == result.transcript_stamp)
        } else {
            None
        };
        self.reconcile_saved_prompts();
        let mut facts = self.session_facts(&result.path);
        if self.parked_snapshot.is_none() && !same_document {
            self.parked_snapshot = Some(std::mem::take(&mut self.snapshot));
        }
        self.project = result.project.clone();
        let previous = if same_document {
            &self.snapshot
        } else {
            self.parked_snapshot.as_ref().unwrap_or(&self.snapshot)
        };
        let auto_retry = previous.auto_retry;
        let models = previous.models.clone();
        let history_stats = historical_context_stats(&history.messages, &models);
        facts.metrics = projection::stable_session_stats(&history_stats, facts.metrics, false);
        let mut session = previous.session.as_ref().filter(|_| same_document).cloned();
        let history_preview = !same_document || previous.history_preview;
        let prefill_model =
            HarnessConfigurationStore::history_model(&models, history.model.as_ref());
        if let Some(session) = &mut session {
            if let Some(model) = &prefill_model {
                session.model = Some(model.clone());
            }
            if let Some(effort) = &history.thinking_level {
                session.thinking_level = Some(effort.clone());
            }
        }
        let mut conversation = ConversationState::default();
        conversation.replace_history(&history.messages);
        drop(projection);
        self.transcript_changed_from = Some(0);
        self.snapshot = RuntimeSnapshot {
            profile_id,
            connected: false,
            document_ready: true,
            saved_history: saved,
            transcript_stamp,
            status: if matches!(
                saved,
                Some(SavedHistory::Partial | SavedHistory::RefreshingPartial)
            ) {
                "Partial saved history".into()
            } else {
                "Ready".into()
            },
            project: result.project,
            selected_session: Some(result.path),
            conversation: Arc::new(conversation),
            models,
            session,
            auto_retry,
            history_preview,
            pending_question: history.pending_question.map(restored_question_request),
            prefill_model,
            prefill_thinking_level: history.thinking_level,
            ..RuntimeSnapshot::default()
        };
        self.snapshot.restore_session_facts(facts);
        self.snapshot.access_mode = self.process_command.access_mode;
        if self.process.is_none() {
            self.restore_saved_session_controls();
        }
        if !refreshing_visible_history {
            let _ = self.event_tx.send(RuntimeEvent::HistoryReset {
                generation: self.process_generation,
            });
        }
        self.publish();
        self.start_pending_document_refresh();
    }

    pub(super) fn start_pending_document_refresh(&mut self) {
        if self.history_selection_generation.is_some() || self.document_refresh_generation.is_some()
        {
            return;
        }
        if let Some((path, project)) = self.pending_document_refresh.take()
            && self.snapshot.document_ready
            && self.snapshot.selected_session.as_ref() == Some(&path)
        {
            self.refresh_session_document(path, project);
        }
    }
}

pub(super) fn annotate_session_prompts(
    store: &SharedStateStore,
    session: TranscriptSession<'_>,
    evidence: Option<&sessions::PromptDeliveryReconciliation>,
    messages: &mut Arc<Vec<Value>>,
) -> Option<farcaster_storage::TranscriptStamp> {
    match store.transcript_prompt_overlay(session, evidence) {
        Ok(overlay) => {
            let stamp = overlay.stamp;
            overlay.apply(messages);
            stamp
        }
        Err(error) => {
            zlog::error!("Read local transcript prompts: {error}");
            None
        }
    }
}

#[cfg(test)]
pub(super) fn annotate_history_presentations(
    state: Option<&StateStore>,
    session: &std::path::Path,
    messages: &mut Arc<Vec<Value>>,
) {
    let Some(state) = state else { return };
    farcaster_storage::PromptHistoryOverlay {
        stamp: None,
        accepted: state.accepted_prompt_history(session).unwrap(),
        presentations: state.prompt_presentations(session).unwrap(),
    }
    .apply(messages);
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
