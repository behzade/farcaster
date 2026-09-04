use super::*;

impl RuntimeOwner {
    pub(super) fn apply_command(&mut self, runtime_command: RuntimeCommand) {
        match runtime_command {
            RuntimeCommand::Prompt {
                target,
                mode,
                message,
                display_message,
                invocation,
                images,
                allow_while_running,
            } => match (display_message, invocation) {
                (None, None) => {
                    self.send_prompt(target, mode, message, images, allow_while_running)
                }
                (display_message, invocation) => self.send_prompt_with_presentation(
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
            RuntimeCommand::Abort => self.send(SessionCommand::Abort),
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
                    &harness,
                    &project,
                    &path,
                    &session_id,
                    &name,
                ) {
                    Ok(()) => self.load_sessions(self.session_query.clone()),
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
                self.harness = harness;
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
                self.harness = harness;
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
                self.harness = harness;
                self.session_id = Some(session_id);
                self.start_process(Some(path));
            }
            RuntimeCommand::RefreshSessionDocument { path, project } => {
                self.refresh_session_document(path, project)
            }
            RuntimeCommand::SetModel(model) => self.set_model(model),
            RuntimeCommand::SetThinking(level) => self.set_thinking(level),
            RuntimeCommand::SetMode(mode) => self.send(SessionCommand::SelectMode { mode }),
            RuntimeCommand::SetAccessMode(mode) => self.set_access_mode(mode),
            RuntimeCommand::SetAppProxy(proxy) => self.set_app_proxy(proxy),
            RuntimeCommand::ExtensionResponse(response) => {
                if let Some(process) = self.process.as_mut()
                    && let Err(error) = process.respond(response)
                {
                    self.fail(error);
                }
            }
            RuntimeCommand::SetSessionArchived { path, archived } => {
                if let Some(state) = &self.state
                    && let Err(error) = sessions::set_archived(state, &path, archived)
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
            RuntimeCommand::ScheduleSessionRefresh => self.schedule_session_refresh(),
            RuntimeCommand::Shutdown => {}
        }
    }
}
