use std::sync::Arc;

use crate::{
    agents::{self, PromptOutcome, QueuedPrompt, SessionCommand},
    protocol::{PromptImage, PromptMode},
};

use super::{RuntimeEvent, RuntimeOwner, can_send_prompt, conversation_mut};

#[cfg(test)]
#[path = "prompts_tests.rs"]
mod tests;

#[derive(Clone, Debug)]
pub(super) struct DeferredPrompt {
    pub(super) mode: PromptMode,
    pub(super) message: String,
    pub(super) display_message: Option<String>,
    pub(super) invocation: Option<String>,
    pub(super) images: Vec<PromptImage>,
    pub(super) outbox_id: Option<i64>,
}

impl RuntimeOwner {
    pub(super) fn send_prompt_for_submission(
        &mut self,
        submission_id: String,
        target: String,
        mode: PromptMode,
        message: String,
        images: Vec<PromptImage>,
        allow_while_running: bool,
    ) {
        self.send_prompt_with_presentation_for_submission(
            submission_id,
            target,
            mode,
            message,
            None,
            None,
            images,
            allow_while_running,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn send_prompt_with_presentation_for_submission(
        &mut self,
        submission_id: String,
        target: String,
        mut mode: PromptMode,
        message: String,
        display_message: Option<String>,
        invocation: Option<String>,
        images: Vec<PromptImage>,
        allow_while_running: bool,
    ) {
        let Some(harness) = self.harness else {
            self.reject_prompt(
                &submission_id,
                &target,
                "Choose a backend before sending a message.".into(),
            );
            return;
        };
        if let Some(error) = self.pending_session_controls.model_error() {
            self.reject_prompt(
                &submission_id,
                &target,
                format!("Select a working model before sending: {error}"),
            );
            return;
        }
        let queue_behind_pending = self.pending_prompt_id.is_some()
            || self.pending_prompt_target.is_some()
            || self.deferred_prompt.is_some();
        if queue_behind_pending && mode == PromptMode::Normal {
            // A normal submission that arrives while another input is in
            // flight is the next follow-up. It is still a valid outbox row;
            // never turn this scheduling fact into a user-facing error.
            mode = PromptMode::FollowUp;
        }
        let was_running = self.active_snapshot().conversation.running;
        if (mode == PromptMode::Normal && self.normal_prompt_in_flight)
            || !can_send_prompt(mode, was_running, allow_while_running)
        {
            self.reject_prompt(
                &submission_id,
                &target,
                format!("{} is already working on this session", self.backend_name()),
            );
            return;
        }
        let queued = self
            .state
            .as_ref()
            .ok_or_else(|| "Couldn’t save the message".to_owned())
            .and_then(|state| {
                state.with(|store| {
                    let images = store.store_prompt_images(&images)?;
                    let id = agents::enqueue_prompt_with_presentation(
                        store,
                        &target,
                        harness,
                        &self.project,
                        self.snapshot.selected_session.as_deref(),
                        mode,
                        &message,
                        display_message.as_deref(),
                        invocation.as_deref(),
                        &images,
                    )?;
                    Ok((Some(id), images))
                })
            });
        let (outbox_id, images) = match queued {
            Ok(queued) => queued,
            Err(error) => {
                self.reject_prompt(&submission_id, &target, error);
                return;
            }
        };
        if queue_behind_pending {
            let outbox_id = outbox_id.expect("saved prompt has an outbox id");
            let can_dispatch_now = self.process.is_some()
                && self.startup_state_loaded
                && self.startup_history_loaded
                && self.active_session.is_some()
                && !self.pending_session_controls.model_pending();
            if can_dispatch_now {
                let dispatch = self
                    .state
                    .as_ref()
                    .ok_or_else(|| "State unavailable".to_owned())
                    .and_then(|state| state.with(|store| agents::begin_prompt(store, outbox_id)))
                    .and_then(|()| {
                        self.process.as_mut().expect("checked process").send(
                            SessionCommand::Prompt {
                                mode,
                                message: message.clone(),
                                images: images.clone(),
                            },
                        )
                    });
                match dispatch {
                    Ok(request_id) => {
                        self.pending_queued_prompts.insert(
                            request_id,
                            super::PendingQueuedPrompt {
                                submission_id,
                                target,
                                outbox_id,
                                session: self.active_session.clone(),
                                delivery_tracked: self
                                    .process
                                    .as_ref()
                                    .is_some_and(|process| process.tracks_prompt_delivery(mode)),
                                result_emitted: false,
                                harness,
                                project: self.project.clone(),
                                mode,
                                message: message.clone(),
                                display_message: display_message.clone(),
                                invocation: invocation.clone(),
                                images: images.clone(),
                            },
                        );
                    }
                    Err(error) => {
                        if is_user_actionable_prompt_error(&error) {
                            self.reject_prompt(&submission_id, &target, error);
                        } else {
                            self.queued_prompts.push_back(QueuedPrompt {
                                id: outbox_id,
                                submission_id: Some(submission_id),
                                target,
                                harness,
                                project: self.project.clone(),
                                session: self.active_session.clone(),
                                mode,
                                message,
                                display_message,
                                invocation,
                                images,
                            });
                        }
                    }
                }
                return;
            }
            self.queued_prompts.push_back(QueuedPrompt {
                id: outbox_id,
                submission_id: Some(submission_id),
                target,
                harness,
                project: self.project.clone(),
                session: self.snapshot.selected_session.clone(),
                mode,
                message,
                display_message,
                invocation,
                images,
            });
            return;
        }
        self.pending_submission_id = Some(submission_id);
        self.pending_prompt_result_emitted = false;
        self.pending_prompt_target = Some(target);
        self.snapshot.pending_question = None;
        let native_invocation = self
            .host
            .contains_invocation(&message, &self.snapshot.commands);
        let conversation = Arc::make_mut(&mut self.snapshot.conversation);
        self.pending_prompt_item = (mode == PromptMode::Normal && !was_running).then(|| {
            match (display_message.as_ref(), invocation.as_ref()) {
                (Some(display), Some(invocation)) => conversation
                    .push_local_invocation_with_prompt_images(
                        display.clone(),
                        &images,
                        invocation.clone(),
                    ),
                _ => conversation.push_local_user_with_prompt_images(
                    message.clone(),
                    &images,
                    native_invocation,
                ),
            }
        });
        conversation.begin_run();
        self.snapshot.status = "Working".into();
        self.publish();
        self.dispatch_prompt(
            mode,
            message,
            display_message,
            invocation,
            images,
            outbox_id,
        );
    }

    #[cfg(test)]
    pub(super) fn send_prompt(
        &mut self,
        target: String,
        mode: PromptMode,
        message: String,
        images: Vec<PromptImage>,
        allow_while_running: bool,
    ) {
        self.send_prompt_for_submission(
            uuid::Uuid::new_v4().to_string(),
            target,
            mode,
            message,
            images,
            allow_while_running,
        );
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn send_prompt_with_presentation(
        &mut self,
        target: String,
        mode: PromptMode,
        message: String,
        display_message: Option<String>,
        invocation: Option<String>,
        images: Vec<PromptImage>,
        allow_while_running: bool,
    ) {
        self.send_prompt_with_presentation_for_submission(
            uuid::Uuid::new_v4().to_string(),
            target,
            mode,
            message,
            display_message,
            invocation,
            images,
            allow_while_running,
        );
    }

    pub(super) fn deliver_queued(&mut self, prompt: QueuedPrompt) {
        if !self.can_deliver_queued(prompt.mode) {
            self.queued_prompts.push_back(prompt);
            return;
        }
        self.project = prompt.project;
        self.snapshot.project = self.project.clone();
        self.snapshot.selected_session = prompt.session.clone();
        self.pending_submission_id = prompt.submission_id;
        self.pending_prompt_result_emitted = false;
        self.pending_prompt_target = Some(prompt.target);
        let native_invocation = self
            .host
            .contains_invocation(&prompt.message, &self.snapshot.commands);
        let conversation = Arc::make_mut(&mut self.snapshot.conversation);
        self.pending_prompt_item = (prompt.mode == PromptMode::Normal).then(|| {
            match (&prompt.display_message, &prompt.invocation) {
                (Some(display), Some(invocation)) => conversation
                    .push_local_invocation_with_prompt_images(
                        display.clone(),
                        &prompt.images,
                        invocation.clone(),
                    ),
                _ => conversation.push_local_user_with_prompt_images(
                    prompt.message.clone(),
                    &prompt.images,
                    native_invocation,
                ),
            }
        });
        conversation.begin_run();
        self.snapshot.status = "Working".into();
        self.publish();
        self.dispatch_prompt(
            prompt.mode,
            prompt.message,
            prompt.display_message,
            prompt.invocation,
            prompt.images,
            Some(prompt.id),
        );
    }

    fn can_deliver_queued(&self, mode: PromptMode) -> bool {
        if self.pending_prompt_id.is_some()
            || self.pending_prompt_target.is_some()
            || self.deferred_prompt.is_some()
        {
            return false;
        }
        if mode != PromptMode::Normal {
            return true;
        }
        let snapshot = self.active_snapshot();
        !self.normal_prompt_in_flight
            && !snapshot.conversation.running
            && !snapshot.conversation.compacting
            && !snapshot.conversation.retrying
            && snapshot.pending_question.is_none()
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_prompt(
        &mut self,
        mode: PromptMode,
        message: String,
        display_message: Option<String>,
        invocation: Option<String>,
        images: Vec<PromptImage>,
        outbox_id: Option<i64>,
    ) {
        if let Some(error) = self
            .pending_session_controls
            .model_error()
            .map(str::to_owned)
        {
            self.pending_outbox_id = outbox_id;
            self.rollback_failed_prompt(&error);
            if let Some(target) = self.pending_prompt_target.take() {
                self.reject_pending_prompt(
                    &target,
                    format!("Select a working model before sending: {error}"),
                );
            }
            return;
        }
        let start_process = self.snapshot.history_preview || self.process.is_none();
        if start_process
            || !self.startup_state_loaded
            || !self.startup_history_loaded
            || self.pending_session_controls.model_pending()
        {
            self.pending_outbox_id = outbox_id;
            self.deferred_prompt = Some(DeferredPrompt {
                mode,
                message,
                display_message,
                invocation,
                images,
                outbox_id,
            });
            if start_process {
                self.start_process(self.snapshot.selected_session.clone());
            }
            return;
        }
        if self.active_session.is_none() {
            let error = format!("{} did not provide a session locator", self.backend_name());
            let error = error.as_str();
            let was_running = self
                .active_snapshot()
                .session
                .as_ref()
                .is_some_and(|state| state.is_streaming);
            self.release_pending_outbox();
            let target = self.pending_prompt_target.take().unwrap_or_default();
            self.rollback_pending_prompt();
            conversation_mut(self.active_snapshot_mut()).running = was_running;
            self.reject_pending_prompt(&target, error.into());
            return;
        }
        if let Some(id) = outbox_id
            && let Some(state) = &self.state
            && let Err(error) = state.with(|store| agents::begin_prompt(store, id))
        {
            let target = self.pending_prompt_target.take().unwrap_or_default();
            self.rollback_pending_prompt();
            self.reject_pending_prompt(&target, error);
            return;
        }
        let was_running = self
            .active_snapshot()
            .session
            .as_ref()
            .is_some_and(|state| state.is_streaming);
        let title_prompt = self
            .should_generate_automatic_title(mode, was_running)
            .then(|| message.clone());
        let request = SessionCommand::Prompt {
            mode,
            message,
            images,
        };
        self.pending_prompt_delivery_tracked = self
            .process
            .as_ref()
            .is_some_and(|process| process.tracks_prompt_delivery(mode));
        // File reads can fail before the backend accepts a request.
        self.pending_outbox_id = outbox_id;
        match self.process.as_mut().map(|process| process.send(request)) {
            Some(Ok(id)) => {
                if let Some(item) = self.pending_prompt_item.clone() {
                    let delivery_tracked = self.pending_prompt_delivery_tracked;
                    conversation_mut(self.active_snapshot_mut())
                        .bind_submitted_prompt_with_evidence(&id, &item, delivery_tracked);
                }
                self.pending_prompt_id = Some(id);
                self.pending_outbox_id = outbox_id;
                self.normal_prompt_in_flight |= mode == PromptMode::Normal;
                if let Some(prompt) = title_prompt {
                    self.start_auto_title_generation(prompt);
                }
            }
            Some(Err(error)) => {
                // Submission may fail locally (for example, an unreadable image or
                // an unsupported mode). Only a transport failure event owns the
                // session lifetime; rejecting this request must not end its turn.
                self.rollback_failed_prompt(&error);
                self.pending_prompt_id = None;
                if let Some(target) = self.pending_prompt_target.take() {
                    self.reject_pending_prompt(&target, error);
                }
            }
            None => {
                // The durable outbox owns retry. A process that is not ready
                // is not a user-facing prompt failure.
                self.release_pending_outbox();
                self.pending_prompt_id = None;
                self.pending_prompt_result_emitted = false;
                self.pending_prompt_delivery_tracked = false;
                self.normal_prompt_in_flight = false;
                if self.parked_snapshot.is_none() {
                    self.publish();
                }
            }
        }
    }

    fn should_generate_automatic_title(&self, mode: PromptMode, was_running: bool) -> bool {
        mode == PromptMode::Normal
            && !was_running
            && self.title_generation.new_session
            && self.startup_state_loaded
            && self.startup_history_loaded
            && self
                .active_snapshot()
                .session
                .as_ref()
                .is_some_and(|state| state.message_count == 0 && state.session_name.is_none())
            && !self.title_generation.in_flight
            && agents::supports_auto_title_generation(self.harness)
    }

    pub(super) fn reject_prompt(&mut self, submission_id: &str, target: &str, message: String) {
        Arc::make_mut(&mut self.snapshot.conversation).push_local_error("Prompt not sent", message);
        self.snapshot.status = "Prompt not sent".into();
        self.emit_prompt_result(
            (!submission_id.is_empty()).then_some(submission_id),
            target,
            PromptOutcome::RejectedBeforeAcceptance,
        );
        self.publish();
    }

    fn reject_pending_prompt(&mut self, target: &str, message: String) {
        let submission_id = self.pending_submission_id.take().unwrap_or_default();
        self.reject_prompt(&submission_id, target, message);
    }

    pub(super) fn rollback_failed_prompt(&mut self, _error: &str) {
        self.release_pending_outbox();
        self.rollback_pending_prompt();
        let running = self
            .active_snapshot()
            .session
            .as_ref()
            .is_some_and(|session| session.is_streaming);
        conversation_mut(self.active_snapshot_mut()).running = running;
    }

    pub(super) fn emit_prompt_result(
        &self,
        submission_id: Option<&str>,
        target: &str,
        mut outcome: PromptOutcome,
    ) {
        let session = self.active_session.clone();
        if outcome == PromptOutcome::Accepted && session.is_none() {
            outcome = PromptOutcome::RejectedBeforeAcceptance;
        }
        let _ = self.event_tx.send(RuntimeEvent::PromptResult {
            submission_id: submission_id.map(str::to_owned),
            target: target.to_owned(),
            outcome,
            session,
        });
    }

    pub(super) fn maybe_send_deferred_prompt(&mut self) {
        if !self.startup_state_loaded
            || !self.startup_history_loaded
            || self.pending_session_controls.model_pending()
            || self.pending_session_controls.model_error().is_some()
        {
            return;
        }
        if self.deferred_prompt.is_none()
            && self
                .queued_prompts
                .front()
                .is_some_and(|prompt| self.can_deliver_queued(prompt.mode))
            && let Some(prompt) = self.queued_prompts.pop_front()
        {
            self.deliver_queued(prompt);
        }
        if let Some(prompt) = self.deferred_prompt.take() {
            if prompt.mode == PromptMode::Normal && self.pending_prompt_item.is_none() {
                let optimistic = match (&prompt.display_message, &prompt.invocation) {
                    (Some(display), Some(invocation)) => {
                        conversation_mut(self.active_snapshot_mut())
                            .push_local_invocation_with_prompt_images(
                                display.clone(),
                                &prompt.images,
                                invocation.clone(),
                            )
                    }
                    _ => {
                        let invocation = self
                            .host
                            .contains_invocation(&prompt.message, &self.active_snapshot().commands);
                        conversation_mut(self.active_snapshot_mut())
                            .push_local_user_with_prompt_images(
                                prompt.message.clone(),
                                &prompt.images,
                                invocation,
                            )
                    }
                };
                self.pending_prompt_item = Some(optimistic);
            }
            let snapshot = self.active_snapshot_mut();
            Arc::make_mut(&mut snapshot.conversation).begin_run();
            snapshot.status = "Working".into();
            if self.snapshot.history_preview
                && let Some(snapshot) = self.parked_snapshot.take()
            {
                self.snapshot = snapshot;
            }
            self.dispatch_prompt(
                prompt.mode,
                prompt.message,
                prompt.display_message,
                prompt.invocation,
                prompt.images,
                prompt.outbox_id,
            );
        }
    }

    pub(super) fn cancel_deferred_prompt(&mut self) {
        let Some(prompt) = self.deferred_prompt.as_ref() else {
            return;
        };
        if let Some(outbox_id) = prompt.outbox_id
            && let Some(state) = &self.state
            && let Err(error) = state.with(|store| store.cancel_queued_prompts(&[outbox_id]))
        {
            zlog::error!("Save deferred prompt cancellation: {error}");
            return;
        }
        self.deferred_prompt = None;
        self.rollback_failed_prompt("Prompt cancelled before delivery");
        if let Some(target) = self.pending_prompt_target.take() {
            self.emit_prompt_result(
                self.pending_submission_id.as_deref(),
                &target,
                PromptOutcome::Cancelled,
            );
        }
        self.pending_submission_id = None;
        self.snapshot.status = "Stopped".into();
        self.publish();
    }
}

fn is_user_actionable_prompt_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "auth",
        "unauthorized",
        "forbidden",
        "permission",
        "access",
        "credential",
        "configuration",
        "configured",
        "config",
        "api key",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}
