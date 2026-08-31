use std::sync::Arc;

use crate::{
    agents::{self, QueuedPrompt, SessionCommand},
    protocol::{PromptImage, PromptMode},
};

use super::{RuntimeEvent, RuntimeOwner, can_send_prompt, conversation_mut};

#[derive(Clone, Debug)]
pub(super) struct DeferredPrompt {
    pub(super) mode: PromptMode,
    pub(super) message: String,
    pub(super) images: Vec<PromptImage>,
    pub(super) outbox_id: Option<i64>,
}

impl RuntimeOwner {
    pub(super) fn send_prompt(
        &mut self,
        target: String,
        mode: PromptMode,
        message: String,
        images: Vec<PromptImage>,
        allow_while_running: bool,
    ) {
        if self.pending_prompt_id.is_some() || self.pending_prompt_target.is_some() {
            self.reject_prompt(&target, "Another message is still being sent".into());
            return;
        }
        let was_running = self.active_snapshot().conversation.running;
        if !can_send_prompt(mode, was_running, allow_while_running) {
            self.reject_prompt(
                &target,
                format!("{} is already working on this session", self.backend_name()),
            );
            return;
        }
        let outbox_id = match self.state.as_ref() {
            Some(state) => match agents::enqueue_prompt(
                state,
                &target,
                &self.harness,
                &self.project,
                self.snapshot.selected_session.as_deref(),
                mode,
                &message,
                &images,
            ) {
                Ok(id) => Some(id),
                Err(error) => {
                    self.reject_prompt(&target, error);
                    return;
                }
            },
            None => {
                self.reject_prompt(&target, "Couldn’t save the message".into());
                return;
            }
        };
        self.pending_prompt_target = Some(target);
        self.snapshot.pending_question = None;
        let invocation =
            crate::app::user_invocations::contains_invocation(&message, &self.snapshot.commands);
        let conversation = Arc::make_mut(&mut self.snapshot.conversation);
        self.pending_prompt_item = (!was_running).then(|| {
            conversation.push_local_user_with_prompt_images(message.clone(), &images, invocation)
        });
        conversation.running = true;
        self.snapshot.status = "Working".into();
        self.publish();
        self.dispatch_prompt(mode, message, images, outbox_id);
    }

    pub(super) fn deliver_queued(&mut self, prompt: QueuedPrompt) {
        self.project = prompt.project;
        self.snapshot.project = self.project.clone();
        self.snapshot.selected_session = prompt.session.clone();
        self.pending_prompt_target = Some(prompt.target);
        let invocation = crate::app::user_invocations::contains_invocation(
            &prompt.message,
            &self.snapshot.commands,
        );
        let conversation = Arc::make_mut(&mut self.snapshot.conversation);
        self.pending_prompt_item = Some(conversation.push_local_user_with_prompt_images(
            prompt.message.clone(),
            &prompt.images,
            invocation,
        ));
        conversation.running = true;
        self.snapshot.status = "Working".into();
        self.publish();
        self.dispatch_prompt(prompt.mode, prompt.message, prompt.images, Some(prompt.id));
    }

    fn dispatch_prompt(
        &mut self,
        mode: PromptMode,
        message: String,
        images: Vec<PromptImage>,
        outbox_id: Option<i64>,
    ) {
        if self.snapshot.history_preview {
            let path = self.snapshot.selected_session.clone();
            self.pending_outbox_id = outbox_id;
            self.deferred_prompt = Some(DeferredPrompt {
                mode,
                message,
                images,
                outbox_id,
            });
            self.start_process(path);
            return;
        }
        if self.process.is_none() {
            self.pending_outbox_id = outbox_id;
            self.deferred_prompt = Some(DeferredPrompt {
                mode,
                message,
                images,
                outbox_id,
            });
            self.start_process(self.snapshot.selected_session.clone());
            return;
        }
        if !self.startup_state_loaded || !self.startup_history_loaded {
            self.pending_outbox_id = outbox_id;
            self.deferred_prompt = Some(DeferredPrompt {
                mode,
                message,
                images,
                outbox_id,
            });
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
            self.mark_outbox_failed(error);
            let target = self.pending_prompt_target.take().unwrap_or_default();
            self.rollback_pending_prompt();
            conversation_mut(self.active_snapshot_mut()).running = was_running;
            self.reject_prompt(&target, error.into());
            return;
        }
        if let Some(id) = outbox_id
            && let Some(state) = &self.state
            && let Err(error) = agents::begin_prompt(state, id)
        {
            let target = self.pending_prompt_target.take().unwrap_or_default();
            self.rollback_pending_prompt();
            self.reject_prompt(&target, error);
            return;
        }
        let request = SessionCommand::Prompt {
            mode,
            message,
            images,
        };
        match self.process.as_mut().map(|process| process.send(request)) {
            Some(Ok(id)) => {
                self.pending_prompt_id = Some(id);
                self.pending_outbox_id = outbox_id;
            }
            Some(Err(error)) => {
                self.mark_outbox_failed(error.as_str());
                self.fail(error);
            }
            None => {
                let error = format!("{} is not connected", self.backend_name());
                self.mark_outbox_failed(&error);
                self.fail(format!("Cannot send prompt: {error}"));
            }
        }
    }

    pub(super) fn reject_prompt(&mut self, target: &str, message: String) {
        Arc::make_mut(&mut self.snapshot.conversation).push_local_error("Prompt not sent", message);
        self.snapshot.status = "Prompt not sent".into();
        self.emit_prompt_result(target, false);
        self.publish();
    }

    pub(super) fn emit_prompt_result(&self, target: &str, accepted: bool) {
        let session = self.active_session.clone();
        let accepted = accepted && session.is_some();
        let _ = self.event_tx.send(RuntimeEvent::PromptResult {
            generation: self.process_generation,
            target: target.to_owned(),
            accepted,
            session,
        });
    }

    pub(super) fn maybe_send_deferred_prompt(&mut self) {
        if !self.startup_state_loaded || !self.startup_history_loaded {
            return;
        }
        if let Some(prompt) = self.deferred_prompt.take() {
            if !crate::agents::is_hidden_text(&prompt.message) && self.pending_prompt_item.is_none()
            {
                let invocation = crate::app::user_invocations::contains_invocation(
                    &prompt.message,
                    &self.active_snapshot().commands,
                );
                let optimistic = conversation_mut(self.active_snapshot_mut())
                    .push_local_user_with_prompt_images(
                        prompt.message.clone(),
                        &prompt.images,
                        invocation,
                    );
                self.pending_prompt_item = Some(optimistic);
            }
            let snapshot = self.active_snapshot_mut();
            Arc::make_mut(&mut snapshot.conversation).running = true;
            snapshot.status = "Working".into();
            if self.snapshot.history_preview
                && let Some(snapshot) = self.parked_snapshot.take()
            {
                self.snapshot = snapshot;
            }
            self.dispatch_prompt(prompt.mode, prompt.message, prompt.images, prompt.outbox_id);
        }
    }
}
