use super::*;

pub(super) fn import_agent_session(session: agents::DiscoveredSession) -> SessionSummary {
    SessionSummary::import(crate::sessions::SessionImport {
        id: session.id,
        harness: session.harness,
        path: session.path,
        project: session.project,
        title: session.title,
        first_user_message: session.first_user_message,
        timestamp: session.timestamp,
        parent_session: session.parent_session,
        modified: session.modified,
        message_count: session.message_count,
        usage: crate::sessions::UsageSummary {
            input: session.usage.input,
            output: session.usage.output,
            cache_read: session.usage.cache_read,
            cache_write: session.usage.cache_write,
            total: session.usage.total,
            cost_micros: session.usage.cost_micros,
        },
        archived: session.archived,
        is_running: session.is_running,
        search: session.search,
    })
}

fn import_agent_history(history: agents::DiscoveredHistory) -> LoadedHistory {
    LoadedHistory {
        messages: history.messages,
        model: history.model,
        thinking_level: history.thinking_level,
        pending_question: None,
    }
}

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

fn load_session_history(path: &std::path::Path) -> Result<LoadedHistory, String> {
    agents::load_external_history(path)
        .map(|result| result.map(import_agent_history))
        .unwrap_or_else(|| sessions::load_history(path))
}

impl RuntimeOwner {
    pub(super) fn select_history(&mut self, path: PathBuf, project: PathBuf) {
        let _timing =
            crate::app::infrastructure::performance::Timing::new("switch.select_document");
        self.history_generation = self.history_generation.saturating_add(1);
        self.pending_document_refresh = None;
        if self.snapshot.selected_session.as_deref() == Some(path.as_path())
            && (self.snapshot.history_preview || self.process.is_some())
        {
            return;
        }
        if self.active_session.as_deref() == Some(path.as_path())
            && (self.process.is_some() || self.parked_snapshot.is_some())
        {
            if let Some(snapshot) = self.parked_snapshot.take() {
                self.snapshot = snapshot;
                self.project = project;
                let _ = self.event_tx.send(RuntimeEvent::HistoryReset {
                    generation: self.process_generation,
                });
                self.publish();
            }
            return;
        }
        self.refresh_history(path, project, HistoryLoadKind::Selection);
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
        if let Err(error) = thread::Builder::new()
            .name("farcaster-history".into())
            .spawn(move || {
                let _timing =
                    crate::app::infrastructure::performance::Timing::new("switch.load_history");
                let mut operation = crate::app::infrastructure::performance::OperationTiming::new(
                    crate::app::infrastructure::performance::OperationKind::HistoryLoad,
                    0,
                );
                let result = load_session_history(&path);
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

    /// Configure an unsubmitted draft without starting its backend.
    pub(super) fn stage_draft(&mut self, harness: String, project: PathBuf) {
        let unchanged = self.process.is_none()
            && self.parked_snapshot.is_none()
            && !self.snapshot.history_preview
            && self.harness == harness
            && self.project == project;
        if unchanged {
            self.publish();
            return;
        }

        self.reset_process_runtime();
        self.harness = harness;
        self.project = project.clone();
        self.session_id = None;
        self.pending_prompt_target = None;
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
        let refreshing_visible_history = result.kind == HistoryLoadKind::DocumentRefresh
            && self.snapshot.history_preview
            && self.snapshot.selected_session.as_ref() == Some(&result.path);
        let mut history = match result.result {
            Ok(history) => history,
            Err(error) => {
                self.snapshot.status = "Could not load history".into();
                conversation_mut(&mut self.snapshot).push_local_error("History unavailable", error);
                self.publish();
                self.start_pending_document_refresh();
                return;
            }
        };
        annotate_history_presentations(self.state.as_ref(), &result.path, &mut history.messages);
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
    messages: &mut [Value],
) {
    let Some(state) = state else { return };
    if let Ok(presentations) = state.prompt_presentations(session) {
        annotate_prompt_presentations(messages, &presentations);
    }
}
