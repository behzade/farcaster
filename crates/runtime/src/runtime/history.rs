use super::*;
use crate::agents::Backend;

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
    pub(super) fn select_history(&mut self, path: PathBuf, project: PathBuf) {
        let _timing = self.host.timer(RuntimeMetric::SelectDocument);
        self.history_generation = self.history_generation.saturating_add(1);
        self.pending_document_refresh = None;
        if self.covers_live_session(&path) {
            if !self.showing_live_session(&path) {
                self.restore_live_session(project);
            }
            return;
        }
        if self.snapshot.selected_session.as_deref() == Some(path.as_path())
            && (self.snapshot.history_preview || self.process.is_some())
        {
            return;
        }
        self.bind_history_selection(path.clone(), project.clone());
        self.refresh_history(path, project, HistoryLoadKind::Selection);
    }

    fn covers_live_session(&self, path: &std::path::Path) -> bool {
        self.active_session.as_deref() == Some(path)
            && (self.process.is_some() || self.parked_snapshot.is_some())
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
        if let Some((harness, session_id)) = agents::external_session_identity(path) {
            self.harness = Some(harness);
            self.session_id = Some(session_id);
        }
    }

    fn bind_history_selection(&mut self, path: PathBuf, project: PathBuf) {
        self.bind_external_session_identity(&path);
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
        self.snapshot.status = "Loading history".into();
        self.snapshot.conversation = Default::default();
        self.transcript_changed_from = Some(0);
        self.publish();
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
        let mut config = self.process_command.clone();
        config.profile_id = agents::profile_id_from_locator(&path);
        let host = self.host.clone();
        if let Err(error) = thread::Builder::new()
            .name("farcaster-history".into())
            .spawn(move || {
                let mut operation = host.timer(RuntimeMetric::LoadHistory);
                let result = harness
                    .ok_or_else(|| "Choose a backend before loading history.".to_owned())
                    .and_then(|harness| {
                        history_cache::load_cached_history(harness, &path, &project)
                    });
                if let Ok(history) = &result {
                    operation.set_work(history.messages.len());
                }
                let _ = sender.send(HistoryResult {
                    generation,
                    path,
                    project,
                    kind,
                    result,
                });
                wake.unpark();
            })
        {
            self.apply_history(HistoryResult {
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
            HistoryLoadKind::Selection => &mut self.history_selection_generation,
            HistoryLoadKind::DocumentRefresh => &mut self.document_refresh_generation,
        }
    }

    pub(super) fn invalidate_history_loads(&mut self) {
        self.history_generation = self.history_generation.saturating_add(1);
        self.history_selection_generation = None;
        self.document_refresh_generation = None;
        self.pending_document_refresh = None;
    }

    pub(super) fn stage_draft(&mut self, id: &str, harness: Option<Backend>, project: PathBuf) {
        let profile_id = match self
            .state
            .as_ref()
            .map(|state| state.with(|store| store.draft_profile_id(id)))
            .transpose()
        {
            Ok(profile_id) => profile_id.flatten(),
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
            && self.process_command.profile_id == profile_id
            && self.project == project;
        if unchanged {
            self.publish();
            return;
        }

        self.reset_process_runtime();
        self.harness = harness;
        self.process_command.profile_id = profile_id;
        self.project = project.clone();
        self.session_id = None;
        self.pending_prompt_target = None;
        self.pending_submission_id = None;
        self.pending_prompt_result_emitted = false;
        self.pending_outbox_id = None;
        self.deferred_prompt = None;
        self.pending_session_controls = PendingSessionControls::default();
        reset_snapshot_for_process(&mut self.snapshot, project, None, "Ready".into());
        self.publish();
    }

    pub(super) fn apply_history(&mut self, result: HistoryResult) {
        let active_generation = self.history_load_generation_mut(result.kind);
        if *active_generation == Some(result.generation) {
            *active_generation = None;
        }
        if result.generation != self.history_generation {
            self.start_pending_document_refresh();
            return;
        }
        if self.covers_live_session(&result.path) {
            self.start_pending_document_refresh();
            return;
        }
        self.bind_external_session_identity(&result.path);
        let refreshing_visible_history = result.kind == HistoryLoadKind::DocumentRefresh
            && self.snapshot.history_preview
            && self.snapshot.selected_session.as_ref() == Some(&result.path);
        let mut history = match result.result {
            Ok(history) => history,
            Err(error) => {
                self.project = result.project.clone();
                self.snapshot.project = result.project;
                self.snapshot.selected_session = Some(result.path);
                self.snapshot.status = "Could not load history".into();
                conversation_mut(&mut self.snapshot).push_local_error("History unavailable", error);
                self.publish();
                self.start_pending_document_refresh();
                return;
            }
        };
        if let (Some(state), Some(evidence)) =
            (self.state.as_ref(), history.prompt_deliveries.as_ref())
            && let Err(error) =
                state.with(|store| store.reconcile_prompt_deliveries(&result.path, evidence))
        {
            zlog::error!("Reconcile saved prompt deliveries: {error}");
        }
        let projection = self.host.timer(RuntimeMetric::ProjectHistory);
        if let Some(state) = self.state.as_ref() {
            if let Err(error) = state.with(|store| {
                annotate_history_presentations(Some(store), &result.path, &mut history.messages);
                Ok(())
            }) {
                zlog::error!("Annotate saved prompt deliveries: {error}");
            }
        }
        if self.parked_snapshot.is_none() {
            self.parked_snapshot = Some(std::mem::take(&mut self.snapshot));
        }
        self.project = result.project.clone();
        let parked = self.parked_snapshot.as_ref();
        let auto_retry = parked.is_some_and(|snapshot| snapshot.auto_retry);
        let models = parked
            .map(|snapshot| snapshot.models.clone())
            .unwrap_or_default();
        let stats = historical_context_stats(&history.messages, &models);
        let prefill_model =
            HarnessConfigurationStore::history_model(&models, history.model.as_ref());
        let mut conversation = ConversationState::default();
        conversation.replace_history(&history.messages);
        drop(projection);
        self.transcript_changed_from = Some(0);
        self.snapshot = RuntimeSnapshot {
            connected: true,
            status: "Ready".into(),
            project: result.project,
            selected_session: Some(result.path),
            conversation: Arc::new(conversation),
            models,
            stats,
            auto_retry,
            history_preview: true,
            pending_question: history.pending_question.map(restored_question_request),
            prefill_model,
            prefill_thinking_level: history.thinking_level,
            ..RuntimeSnapshot::default()
        };
        self.snapshot.access_mode = self.process_command.access_mode;
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
            && self.snapshot.history_preview
            && self.snapshot.selected_session.as_ref() == Some(&path)
        {
            self.refresh_session_document(path, project);
        }
    }
}

pub(super) fn annotate_history_presentations(
    state: Option<&StateStore>,
    session: &std::path::Path,
    messages: &mut Vec<Value>,
) {
    let Some(state) = state else { return };
    match state.accepted_prompt_history(session) {
        Ok(saved) => {
            let history_was_empty = messages.is_empty();
            let mut submission_ids = messages
                .iter()
                .filter_map(|message| message.get("submissionId").and_then(Value::as_str))
                .map(str::to_owned)
                .collect::<std::collections::HashSet<_>>();
            for message in &saved {
                let Some(id) = message.get("submissionId").and_then(Value::as_str) else {
                    if history_was_empty {
                        messages.push(message.clone());
                    }
                    continue;
                };
                // Untracked backends never emit a later delivery id. If native
                // history loaded, it is the source of truth — including steers.
                if !history_was_empty
                    && message.get("deliveryTracked").and_then(Value::as_bool) != Some(true)
                {
                    continue;
                }
                if submission_ids.insert(id.to_owned()) {
                    messages.push(message.clone());
                }
            }
        }
        Err(error) => {
            zlog::error!("Restore accepted prompts: {error}");
        }
    }
    if let Ok(presentations) = state.prompt_presentations(session) {
        annotate_prompt_presentations(messages, &presentations);
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
