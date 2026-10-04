use super::*;

impl RuntimeOwner {
    pub(super) fn apply_signal(&mut self, signal: RuntimeSignal) {
        let command = match signal {
            RuntimeSignal::Abort => {
                self.cancel_recovered_prompts();
                self.cancel_deferred_prompt();
                if !self.active_snapshot().conversation.running && !self.normal_prompt_in_flight {
                    return;
                }
                SessionCommand::Abort
            }
            RuntimeSignal::ApplySteering => {
                let pending_steer = self.pending_prompt.as_ref().is_some_and(|prompt| {
                    matches!(
                        prompt.phase,
                        PromptPhase::Dispatched {
                            mode: PromptMode::Steer,
                            delivered: false,
                            ..
                        }
                    )
                }) || self
                    .pending_queued_prompts
                    .values()
                    .any(|prompt| prompt.mode == PromptMode::Steer && !prompt.result_emitted);
                let queue = &self.active_snapshot().conversation.queue;
                if !pending_steer && queue.steering.is_empty() {
                    return;
                }
                SessionCommand::ApplySteering
            }
        };
        let Some(process) = self.process.as_mut() else {
            return;
        };
        match process.send(command) {
            Ok(id) => {
                self.idle_retirement.invalidate();
                self.idle_retirement.requests.insert(id);
            }
            // Startup, settlement, or another signal can make this signal stale.
            // Transport failures still arrive through the event stream.
            Err(error) => {
                zlog::debug!("Signal was not applied: {error}");
            }
        }
    }
}
