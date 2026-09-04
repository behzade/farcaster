use super::*;

impl RuntimeOwner {
    pub(super) fn start_auto_title_generation(&mut self, prompt: String) {
        if self.title_generation.in_flight
            || !agents::supports_auto_title_generation(&self.harness)
            || self
                .active_snapshot()
                .session
                .as_ref()
                .and_then(|state| state.session_name.as_ref())
                .is_some()
        {
            return;
        }
        let generation = self.process_generation;
        let revision = self.title_generation.revision;
        let active_model = self
            .active_snapshot()
            .session
            .as_ref()
            .and_then(|session| session.model.clone());
        let config = self.process_command.clone();
        let harness = self.harness.clone();
        let project = self.project.clone();
        let sender = self.title_generation.sender.clone();
        let wake = thread::current();
        self.title_generation.in_flight = true;
        if let Err(error) = thread::Builder::new()
            .name("farcaster-session-title".into())
            .spawn(move || {
                let result = agents::generate_session_title(
                    &config,
                    &harness,
                    &project,
                    &prompt,
                    active_model.as_ref(),
                );
                let _ = sender.send(SessionTitleResult {
                    generation,
                    revision,
                    result,
                });
                wake.unpark();
            })
        {
            self.title_generation.in_flight = false;
            zlog::warn!("Failed to start session title generation: {error}");
        }
    }

    pub(super) fn invalidate_auto_title_generation(&mut self) {
        self.title_generation.in_flight = false;
        self.title_generation.revision = self.title_generation.revision.saturating_add(1);
    }

    pub(super) fn apply_generated_session_title(&mut self, result: SessionTitleResult) {
        if result.generation != self.process_generation {
            return;
        }
        if result.revision != self.title_generation.revision {
            return;
        }
        self.title_generation.in_flight = false;
        let title = match result.result {
            Ok(title) => title,
            Err(error) => {
                zlog::warn!("Session title generation failed: {error}");
                return;
            }
        };
        let unnamed = self
            .active_snapshot()
            .session
            .as_ref()
            .is_some_and(|state| state.session_name.is_none());
        if unnamed {
            if let Some(state) = self.active_snapshot_mut().session.as_mut() {
                state.session_name = Some(title.clone());
            }
            self.send(SessionCommand::Rename { name: title });
            self.refresh_sessions();
        }
    }

    pub(super) fn backend_name(&self) -> &str {
        match self.harness.as_str() {
            "pi" => "Pi",
            "codex-cli" => "Codex",
            "cursor-cli" => "Cursor",
            "opencode2" => "OpenCode",
            other => other,
        }
    }

    pub(super) fn start_process(&mut self, session: Option<PathBuf>) {
        self.start_process_from(session, None, false);
    }

    pub(super) fn restart_process_preserving_transcript(&mut self) {
        let session = if self.snapshot.history_preview {
            self.snapshot.selected_session.clone()
        } else {
            self.active_session.clone()
        };
        self.start_process_from(session, None, true);
    }

    pub(super) fn start_fork_process(&mut self, source: PathBuf) {
        self.start_process_from(None, Some(source), false);
    }

    pub(super) fn reset_process_runtime(&mut self) {
        self.invalidate_history_loads();
        self.process_generation = self.process_generation.saturating_add(1);
        if let Some(mut process) = self.process.take() {
            let _ = process.close();
        }
        self.active_session = None;
        self.parked_snapshot = None;
        self.startup_state_loaded = false;
        self.startup_history_loaded = false;
        self.pending_prompt_id = None;
        self.pending_prompt_item = None;
        self.invalidate_auto_title_generation();
        self.transcript_changed_from = Some(0);
    }

    pub(super) fn start_process_from(
        &mut self,
        session: Option<PathBuf>,
        fork: Option<PathBuf>,
        preserve_transcript: bool,
    ) {
        let preserve_transcript = preserve_transcript
            || self.deferred_prompt.is_some()
            || (!self.pending_session_controls.is_empty() && self.snapshot.history_preview);
        let keep_preview = preserve_transcript && self.snapshot.history_preview;
        let preserved_conversation =
            (preserve_transcript && !keep_preview).then(|| self.snapshot.conversation.clone());
        let preserved_prompt_item = preserved_conversation
            .as_ref()
            .and(self.pending_prompt_item.clone());
        self.reset_process_runtime();
        self.active_session = session.clone();
        self.process_command.access_mode = self
            .access_mode_changes
            .take_requested_mode(self.process_command.access_mode);
        let status = if fork.is_some() {
            "Forking session".into()
        } else {
            session.as_ref().map_or_else(
                || "Starting new session".into(),
                |_| "Resuming session".into(),
            )
        };
        if keep_preview {
            let mut loading = RuntimeSnapshot {
                auto_retry: self.snapshot.auto_retry,
                ..RuntimeSnapshot::default()
            };
            reset_snapshot_for_process(&mut loading, self.project.clone(), session.clone(), status);
            self.parked_snapshot = Some(loading);
        } else {
            reset_snapshot_for_process(
                &mut self.snapshot,
                self.project.clone(),
                session.clone(),
                status,
            );
            if let Some(conversation) = preserved_conversation {
                self.snapshot.conversation = conversation;
                self.pending_prompt_item = preserved_prompt_item;
            }
        }
        let _ = self.event_tx.send(RuntimeEvent::SessionReset {
            generation: self.process_generation,
            preserve_submission: preserve_transcript,
        });
        self.publish();
        let start = if let Some(source) = fork {
            SessionStart::Fork(source)
        } else if let Some(session) = session {
            SessionStart::Resume(session)
        } else {
            SessionStart::New
        };
        self.process_command.access_mode =
            crate::agents::normalize_access_mode(&self.harness, self.process_command.access_mode);
        let process = crate::agents::spawn_session(
            &self.process_command,
            SessionLaunch {
                harness: self.harness.clone(),
                session_id: self.session_id.clone(),
                project: self.project.clone(),
                start,
                wake: Some(thread::current()),
            },
        );
        match process {
            Ok(process) => {
                self.process = Some(process);
                let snapshot = self.active_snapshot_mut();
                snapshot.connected = true;
                snapshot.status = "Loading session".into();
                self.send_startup_queries();
            }
            Err(error) => self.fail(error),
        }
        self.publish();
    }

    pub(super) fn send_startup_queries(&mut self) {
        for command in startup_commands() {
            if agents::supports_startup_command(&self.harness, &command) {
                self.send(command);
            }
        }
    }

    pub(super) fn reload(&mut self) {
        let active = self.active_snapshot();
        if active.conversation.running || active.conversation.compacting {
            let snapshot = self.active_snapshot_mut();
            conversation_mut(snapshot).push_local_error(
                "Reload not started",
                "Wait for the current response to finish before reloading.".into(),
            );
            snapshot.status = "Reload not started".into();
            self.publish();
            return;
        }
        let session = if self.snapshot.history_preview {
            self.snapshot.selected_session.clone()
        } else {
            self.active_session.clone()
        };
        self.start_process(session);
    }

    pub(super) fn send(&mut self, request: SessionCommand) {
        let operation = request.operation();
        match self.process.as_mut().map(|process| process.send(request)) {
            Some(Ok(_)) => {}
            Some(Err(error)) => self.fail(error),
            None => self.fail(format!(
                "Cannot {operation}: {} is not connected",
                self.backend_name()
            )),
        }
    }

    pub(super) fn apply_process_item(&mut self, item: SessionEvent) -> SnapshotChange {
        match item {
            SessionEvent::Response(response) => {
                self.apply_response(response);
                SnapshotChange::None
            }
            SessionEvent::Interaction(request) => {
                let _ = self.event_tx.send(RuntimeEvent::ExtensionUi {
                    generation: self.process_generation,
                    request,
                    system_notification_target: None,
                });
                SnapshotChange::None
            }
            SessionEvent::Activity(event) => {
                let settled = event.kind() == &SessionActivityKind::AgentSettled;
                let session_starting = event.kind() == &SessionActivityKind::AgentStarted
                    && self.active_session.is_none()
                    && self.parked_snapshot.is_none();
                let previewing = self.parked_snapshot.is_some();
                let previous_live_status =
                    previewing.then(|| session_badge_status(&self.active_snapshot().conversation));
                let (changed_from, snapshot_changed, live_status_changed) = {
                    let snapshot = self.active_snapshot_mut();
                    let (changed_from, conversation_state_changed) =
                        conversation_mut(snapshot).reduce_deferred_with_change(event.value());
                    let context_changed =
                        update_context_from_event(&mut snapshot.stats, event.value());
                    let goal_changed = update_session_goal_from_event(
                        &mut snapshot.session_goal,
                        event.kind(),
                        event.value(),
                    );
                    let status = run_status(&snapshot.conversation);
                    let status_changed = snapshot.status != status;
                    snapshot.status = status.to_owned();
                    let live_status_changed = previous_live_status.is_some_and(|status| {
                        status != session_badge_status(&snapshot.conversation)
                    });
                    (
                        changed_from,
                        changed_from.is_some()
                            || conversation_state_changed
                            || context_changed
                            || goal_changed
                            || status_changed,
                        live_status_changed,
                    )
                };
                if let Some(changed_from) = changed_from {
                    self.transcript_changed_from = Some(
                        self.transcript_changed_from
                            .map_or(changed_from, |previous| previous.min(changed_from)),
                    );
                }
                let should_publish = (!previewing && snapshot_changed) || live_status_changed;
                if session_starting {
                    self.send(SessionCommand::LoadState);
                }
                if event.kind() == &SessionActivityKind::AgentStarted {
                    self.refresh_sessions();
                }
                if event.kind() == &SessionActivityKind::SessionChanged {
                    self.send(SessionCommand::LoadState);
                    self.refresh_sessions();
                }
                if tool_starts_worker(event.kind(), event.value()) {
                    self.schedule_session_refresh();
                }
                if settled {
                    self.send(SessionCommand::LoadState);
                    self.send(SessionCommand::LoadUsage);
                    self.refresh_sessions();
                }
                if !should_publish {
                    SnapshotChange::None
                } else if matches!(
                    event.kind(),
                    SessionActivityKind::MessageUpdated | SessionActivityKind::ToolUpdated
                ) {
                    SnapshotChange::Streaming
                } else {
                    SnapshotChange::Immediate
                }
            }
            SessionEvent::Stderr(chunk) => {
                let previewing = self.parked_snapshot.is_some();
                let snapshot = self.active_snapshot_mut();
                snapshot.stderr.push_str(&chunk);
                if snapshot.stderr.len() > 32 * 1024 {
                    snapshot.stderr.drain(..16 * 1024);
                }
                if previewing {
                    SnapshotChange::None
                } else {
                    SnapshotChange::Streaming
                }
            }
            SessionEvent::Failure(error) => {
                self.fail(error);
                SnapshotChange::None
            }
        }
    }

    pub(super) fn active_snapshot_mut(&mut self) -> &mut RuntimeSnapshot {
        self.parked_snapshot.as_mut().unwrap_or(&mut self.snapshot)
    }

    pub(super) fn active_snapshot(&self) -> &RuntimeSnapshot {
        self.parked_snapshot.as_ref().unwrap_or(&self.snapshot)
    }

    pub(super) fn rollback_pending_prompt(&mut self) {
        if let Some(optimistic) = self.pending_prompt_item.take() {
            conversation_mut(self.active_snapshot_mut()).rollback_local_user(&optimistic);
        }
    }

    pub(super) fn fail(&mut self, error: String) {
        let starting = !self.startup_state_loaded || !self.startup_history_loaded;
        let preserve_history = !self.pending_session_controls.is_empty()
            && self.snapshot.history_preview
            && self.parked_snapshot.is_some();
        let details = failure_details(&error);
        zlog::error!("agent runtime failed: {details}");
        self.mark_outbox_failed(&details);
        self.pending_prompt_id = None;
        self.deferred_prompt = None;
        self.process_command.access_mode = self
            .access_mode_changes
            .take_requested_mode(self.process_command.access_mode);
        self.rollback_pending_prompt();
        if let Some(target) = self.pending_prompt_target.take() {
            self.emit_prompt_result(&target, false);
        }
        if preserve_history {
            let label = format!("Couldn’t start {}", self.backend_name());
            self.fail_session_control_resume("Failed", &label, details);
            return;
        }
        self.pending_session_controls = PendingSessionControls::default();
        if let Some(mut process) = self.process.take() {
            let _ = process.close();
        }
        let previewing = self.parked_snapshot.is_some();
        let label = if starting {
            format!("Couldn’t start {}", self.backend_name())
        } else {
            format!("{} stopped", self.backend_name())
        };
        let snapshot = self.active_snapshot_mut();
        snapshot.connected = false;
        snapshot.status = "Failed".into();
        let conversation = conversation_mut(snapshot);
        conversation.diagnostics.push(details.clone());
        conversation.push_local_error_with_details(&label, failure_summary(&details), details);
        if previewing && let Some(snapshot) = self.parked_snapshot.take() {
            self.snapshot = snapshot;
        }
        self.publish();
    }

    pub(super) fn mark_outbox_failed(&mut self, error: &str) {
        if let Some(id) = self.pending_outbox_id.take()
            && let Some(state) = &self.state
            && let Err(database_error) = agents::fail_prompt(state, id, error)
        {
            zlog::error!("Failed to mark queued prompt {id} as failed: {database_error}");
        }
    }

    pub(super) fn publish(&mut self) {
        crate::app::infrastructure::performance::count_snapshot();
        self.snapshot.access_mode = self
            .access_mode_changes
            .requested_mode(self.process_command.access_mode);
        conversation_mut(self.active_snapshot_mut()).flush_live_projection();
        let active_snapshot = self.active_snapshot();
        let mut snapshot = self.snapshot.clone();
        snapshot.harness.clone_from(&self.harness);
        snapshot.live_session = self
            .active_session
            .clone()
            .or_else(|| active_snapshot.selected_session.clone());
        snapshot.live_status = session_badge_status(&active_snapshot.conversation).into();
        snapshot.transcript_changed_from = self.transcript_changed_from.take();
        let _ = self.event_tx.send(RuntimeEvent::Snapshot {
            generation: self.process_generation,
            snapshot: Arc::new(snapshot),
        });
    }
}

pub(super) fn conversation_mut(snapshot: &mut RuntimeSnapshot) -> &mut ConversationState {
    Arc::make_mut(&mut snapshot.conversation)
}

pub(super) fn reset_snapshot_for_process(
    snapshot: &mut RuntimeSnapshot,
    project: PathBuf,
    selected_session: Option<PathBuf>,
    status: String,
) {
    let auto_retry = snapshot.auto_retry;
    *snapshot = RuntimeSnapshot {
        status,
        project,
        selected_session,
        auto_retry,
        ..RuntimeSnapshot::default()
    };
}

pub(super) fn startup_commands() -> [SessionCommand; 8] {
    [
        SessionCommand::ConfigureSteering,
        SessionCommand::LoadState,
        SessionCommand::LoadHistory,
        SessionCommand::LoadUsage,
        SessionCommand::ListModels,
        SessionCommand::ListReasoningLevels,
        SessionCommand::ListModes,
        SessionCommand::ListCommands,
    ]
}

pub(super) const fn can_send_prompt(
    mode: PromptMode,
    running: bool,
    allow_while_running: bool,
) -> bool {
    allow_while_running || !running || !matches!(mode, PromptMode::Normal)
}
