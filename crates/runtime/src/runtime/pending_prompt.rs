use super::*;

#[derive(Clone, Debug)]
pub(super) struct PromptInput {
    pub mode: PromptMode,
    pub message: String,
    pub display_message: Option<String>,
    pub invocation: Option<String>,
    pub images: Vec<PromptImage>,
}

pub(super) enum PromptPhase {
    Waiting(PromptInput),
    Dispatched {
        mode: PromptMode,
        request_id: String,
        delivery_tracked: bool,
        delivered: bool,
    },
}

pub(super) struct PendingPrompt {
    pub submission_id: Option<String>,
    pub target: String,
    pub outbox_id: Option<i64>,
    pub item: Option<Arc<TranscriptItem>>,
    pub phase: PromptPhase,
}

impl PendingPrompt {
    pub fn request_id(&self) -> Option<&str> {
        match &self.phase {
            PromptPhase::Waiting(_) => None,
            PromptPhase::Dispatched { request_id, .. } => Some(request_id),
        }
    }

    pub fn waiting(&self) -> Option<&PromptInput> {
        match &self.phase {
            PromptPhase::Waiting(input) => Some(input),
            PromptPhase::Dispatched { .. } => None,
        }
    }

    pub fn delivered(&self) -> bool {
        matches!(
            self.phase,
            PromptPhase::Dispatched {
                delivered: true,
                ..
            }
        )
    }

    pub fn delivery_tracked(&self) -> bool {
        matches!(
            self.phase,
            PromptPhase::Dispatched {
                delivery_tracked: true,
                ..
            }
        )
    }

    pub fn mark_dispatched(&mut self, request_id: String, delivery_tracked: bool) {
        let mode = self.waiting().expect("waiting prompt").mode;
        self.phase = PromptPhase::Dispatched {
            mode,
            request_id,
            delivery_tracked,
            delivered: false,
        };
    }

    pub fn mark_delivered(&mut self) {
        if let PromptPhase::Dispatched { delivered, .. } = &mut self.phase {
            *delivered = true;
        }
    }

    pub fn receipt(&self, session: Option<PathBuf>) -> prompt_receipts::RetiredPrompt {
        prompt_receipts::RetiredPrompt {
            outbox_id: self.outbox_id,
            target: self.target.clone(),
            session,
            delivery_tracked: self.delivery_tracked(),
            submission_id: self.submission_id.clone(),
            delivered: self.delivered(),
        }
    }
}

impl RuntimeOwner {
    pub(super) fn pending_request_id(&self) -> Option<&str> {
        self.pending_prompt
            .as_ref()
            .and_then(PendingPrompt::request_id)
    }

    pub(super) fn deferred_prompt(&self) -> Option<&PromptInput> {
        self.pending_prompt
            .as_ref()
            .and_then(PendingPrompt::waiting)
    }

    pub(super) fn rollback_pending_prompt(&mut self) {
        if let Some(optimistic) = self
            .pending_prompt
            .as_mut()
            .and_then(|prompt| prompt.item.take())
        {
            conversation_mut(self.active_snapshot_mut()).rollback_local_user(&optimistic);
        }
    }

    pub(super) fn finish_current_prompt(&mut self, rollback: bool) {
        if rollback {
            self.rollback_pending_prompt();
        }
        // A failed receipt write must keep its delivery proof after current work ends.
        if let Some(prompt) = self.pending_prompt.take()
            && prompt.delivered()
            && prompt.outbox_id.is_some()
            && let Some(id) = prompt.request_id()
        {
            self.retired_prompts
                .insert(id.to_owned(), prompt.receipt(self.active_session.clone()));
        }
    }
}
