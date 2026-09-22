use super::*;

impl RuntimeOwner {
    fn clear_prompt_queue(&mut self) -> Result<(), String> {
        self.process
            .as_mut()
            .ok_or("No live queue is connected")?
            .clear_queue()?;
        let local = self
            .queued_prompts
            .iter()
            .filter_map(|prompt| Some((prompt.submission_id.clone()?, prompt.target.clone())))
            .collect::<Vec<_>>();
        self.cancel_recovered_prompts();
        for (id, target) in local {
            self.emit_prompt_result(Some(&id), &target, agents::PromptOutcome::Cancelled);
        }
        // Only the queue owner knows which inputs it has already claimed.
        // Persist its exact cancellation events; never mark every in-flight
        // prompt cancelled merely because the pending queue was cleared.
        Ok(())
    }

    fn cancel_queued_prompt(&mut self, target: &str, id: &str) -> Result<(), String> {
        if let Some(index) = self.queued_prompts.iter().position(|prompt| {
            prompt.target == target && prompt.submission_id.as_deref() == Some(id)
        }) {
            let prompt = &self.queued_prompts[index];
            self.state
                .as_ref()
                .ok_or("State unavailable")?
                .with(|store| store.cancel_queued_prompts(&[prompt.id]))?;
            self.queued_prompts.remove(index);
            self.emit_prompt_result(Some(id), target, agents::PromptOutcome::Cancelled);
            return Ok(());
        }
        let request_id = self
            .pending_queued_prompts
            .iter()
            .find_map(|(request_id, prompt)| {
                (prompt.target == target && (request_id == id || prompt.submission_id == id))
                    .then(|| request_id.clone())
            })
            .or_else(|| {
                (self.pending_prompt_target.as_deref() == Some(target)
                    && (self.pending_prompt_id.as_deref() == Some(id)
                        || self.pending_submission_id.as_deref() == Some(id)))
                .then(|| self.pending_prompt_id.clone())
                .flatten()
            });
        if let (Some(request_id), Some(process)) = (request_id, self.process.as_mut()) {
            // The shared queue owns cancellation and resolves stale clicks.
            process.cancel_prompt(&request_id)?;
        }
        Ok(())
    }

    fn cancel_recovered_prompts(&mut self) {
        if self.queued_prompts.is_empty() {
            return;
        }
        let ids = self
            .queued_prompts
            .iter()
            .map(|prompt| prompt.id)
            .collect::<Vec<_>>();
        let result = self
            .state
            .as_ref()
            .ok_or_else(|| "State unavailable".to_owned())
            .and_then(|state| state.with(|store| store.cancel_queued_prompts(&ids)));
        // Stop this run even if the disk write fails. In that case make the
        // missing durability explicit: we cannot promise safety after restart.
        self.queued_prompts.clear();
        if let Err(error) = result {
            zlog::error!("Could not persist queue cancellation: {error}");
        }
    }

    pub(super) fn apply_command(&mut self, runtime_command: RuntimeCommand) {
        match runtime_command {
            RuntimeCommand::ClearQueue => {
                if let Err(error) = self.clear_prompt_queue() {
                    zlog::error!("Could not clear queue: {error}");
                }
                self.publish();
            }
            RuntimeCommand::CancelQueued { target, id } => {
                if let Err(error) = self.cancel_queued_prompt(&target, &id) {
                    zlog::error!("Could not cancel queued prompt: {error}");
                }
                self.publish();
            }
            RuntimeCommand::DismissReceipt { session, id } => {
                if self.snapshot.selected_session.as_ref() != Some(&session)
                    || !self.snapshot.history_preview
                {
                    return;
                }
                let result = self
                    .state
                    .as_ref()
                    .ok_or_else(|| "State unavailable".to_owned())
                    .and_then(|state| {
                        state.with(|store| store.dismiss_prompt_receipt(&session, &id))
                    });
                match result {
                    Ok(()) => conversation_mut(&mut self.snapshot).dismiss_pending_receipt(&id),
                    Err(error) => {
                        zlog::error!("Could not dismiss prompt receipt: {error}");
                    }
                }
                self.publish();
            }
            RuntimeCommand::SendToSession {
                submission_id,
                target,
                message,
                ..
            } => {
                let mode = if self.active_snapshot().conversation.running {
                    if agents::supports_steering(self.harness) {
                        PromptMode::Steer
                    } else {
                        PromptMode::FollowUp
                    }
                } else {
                    PromptMode::Normal
                };
                self.send_prompt_for_submission(
                    submission_id,
                    target,
                    mode,
                    message,
                    Vec::new(),
                    false,
                );
            }
            RuntimeCommand::Prompt {
                submission_id,
                target,
                mode,
                message,
                display_message,
                invocation,
                images,
                allow_while_running,
            } => match (display_message, invocation) {
                (None, None) => self.send_prompt_for_submission(
                    submission_id,
                    target,
                    mode,
                    message,
                    images,
                    allow_while_running,
                ),
                (display_message, invocation) => self.send_prompt_with_presentation_for_submission(
                    submission_id,
                    target,
                    mode,
                    message,
                    display_message,
                    invocation,
                    images,
                    allow_while_running,
                ),
            },
            RuntimeCommand::DeliverQueued(prompt) => self.deliver_queued(prompt),
            RuntimeCommand::UpdateConfigurationCatalog {
                harness,
                project,
                catalog,
            } => {
                if self.harness == Some(harness) && self.project == project {
                    let mut changed = false;
                    for snapshot in
                        std::iter::once(&mut self.snapshot).chain(self.parked_snapshot.iter_mut())
                    {
                        if snapshot.models != catalog.models {
                            snapshot.models.clone_from(&catalog.models);
                            changed = true;
                        }
                        if snapshot.thinking_levels != catalog.efforts {
                            snapshot.thinking_levels.clone_from(&catalog.efforts);
                            changed = true;
                        }
                        if !snapshot.connected
                            && snapshot.sandbox_adapter != catalog.sandbox_adapter
                        {
                            snapshot
                                .sandbox_adapter
                                .clone_from(&catalog.sandbox_adapter);
                            changed = true;
                        }
                        if snapshot.configuration_status != ConfigurationStatus::Loaded {
                            snapshot.configuration_status = ConfigurationStatus::Loaded;
                            changed = true;
                        }
                    }
                    if changed {
                        self.publish();
                    }
                }
            }
            RuntimeCommand::Abort => {
                self.cancel_recovered_prompts();
                self.cancel_deferred_prompt();
                self.send(SessionCommand::Abort);
            }
            RuntimeCommand::ApplySteering => self.send(SessionCommand::ApplySteering),
            RuntimeCommand::Reload => self.reload(),
            RuntimeCommand::Compact {
                custom_instructions,
            } => self.send(SessionCommand::Compact {
                instructions: custom_instructions,
            }),
            RuntimeCommand::ExportHtml { output_path } => {
                self.send(SessionCommand::ExportHtml { output_path })
            }
            RuntimeCommand::SetSessionName(name) => {
                self.invalidate_auto_title_generation();
                if let Some(state) = self.active_snapshot_mut().session.as_mut() {
                    state.session_name = Some(name.clone());
                }
                self.send(SessionCommand::Rename { name })
            }
            RuntimeCommand::RenameSession {
                path,
                harness,
                session_id,
                project,
                name,
            } => {
                match crate::agents::rename_session(
                    &self.process_command,
                    harness,
                    &project,
                    &path,
                    &session_id,
                    &name,
                ) {
                    Ok(()) => self.update_session_metadata(agents::SessionMetadata {
                        harness,
                        id: session_id,
                        path,
                        project,
                        title: Some(name),
                        first_user_message: None,
                        parent_session: None,
                        message_count: None,
                        model: None,
                        thinking_level: None,
                        service_tier: None,
                        access_mode: None,
                        usage: None,
                        is_running: false,
                    }),
                    Err(message) => {
                        let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                            generation: self.session_generation,
                            message,
                        });
                    }
                }
            }
            RuntimeCommand::MoveSession { .. }
            | RuntimeCommand::StopSessionFamily { .. }
            | RuntimeCommand::DeleteSessionFamily { .. } => {}
            RuntimeCommand::NewSession {
                harness, project, ..
            } => self.stage_draft(harness, project),
            RuntimeCommand::ForkSession {
                path,
                harness,
                session_id,
                project,
            } => {
                self.project = project;
                self.harness = Some(harness);
                self.session_id = Some(session_id);
                self.start_fork_process(path);
            }
            RuntimeCommand::ResumeDraft {
                harness, project, ..
            } => self.stage_draft(harness, project),
            RuntimeCommand::SelectSession {
                path,
                harness,
                session_id,
                project,
            } => {
                self.harness = Some(harness);
                self.session_id = Some(session_id);
                self.select_history(path, project);
            }
            RuntimeCommand::RestartSession {
                path,
                harness,
                session_id,
                project,
            } => {
                self.project = project;
                self.harness = Some(harness);
                self.session_id = Some(session_id);
                self.start_process(Some(path));
            }
            RuntimeCommand::RefreshSessionDocument {
                path,
                project,
                harness,
            } => {
                if harness.is_some() {
                    self.harness = harness;
                }
                self.bind_external_session_identity(&path);
                self.refresh_session_document(path, project)
            }
            RuntimeCommand::SetModel(model) => self.set_model(model),
            RuntimeCommand::SetThinking(level) => self.set_thinking(level),
            RuntimeCommand::ResetThinking => self.reset_thinking(),
            RuntimeCommand::SetServiceTier(tier) => self.set_service_tier(tier),
            RuntimeCommand::SetAccessMode(mode) => self.set_access_mode(mode),
            RuntimeCommand::RestoreAccessMode(mode) => {
                self.process_command.access_mode = mode;
                self.access_mode_changes = Default::default();
                self.snapshot.access_mode = mode;
            }
            RuntimeCommand::SetAppProxy(proxy) => self.set_app_proxy(proxy),
            RuntimeCommand::ExtensionResponse(response) => {
                if self.respond_to_child_input(&response) {
                    return;
                }
                if let Some(process) = self.process.as_mut()
                    && let Err(error) = process.respond(response)
                {
                    self.fail(error);
                }
            }
            RuntimeCommand::SetSessionArchived { path, archived } => {
                if let Some(state) = &self.state
                    && let Err(error) =
                        state.with(|store| sessions::set_archived(store, &path, archived))
                {
                    let _ = self.event_tx.send(RuntimeEvent::SessionsFailed {
                        generation: self.session_generation,
                        message: error,
                    });
                }
                self.load_sessions(self.session_query.clone());
            }
            RuntimeCommand::LoadSessions(query) => self.load_sessions(query),
            RuntimeCommand::RefreshSessions => self.refresh_sessions(),
            RuntimeCommand::UpdateSessionMetadata(metadata) => {
                self.update_session_metadata(metadata)
            }
            RuntimeCommand::ScheduleSessionRefresh => self.schedule_session_refresh(),
            RuntimeCommand::PreviewImport {
                harness,
                generation,
            } => self.preview_import(harness, generation),
            RuntimeCommand::CommitImport { sessions } => self.commit_import(sessions),
            // Configuration loading belongs to the supervisor, not a chat actor.
            RuntimeCommand::LoadConfiguration { .. }
            | RuntimeCommand::StartTask { .. }
            | RuntimeCommand::Shutdown => {}
        }
    }
}
