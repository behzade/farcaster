use super::*;

#[derive(Clone)]
pub(super) struct RetiredPrompt {
    pub(super) outbox_id: Option<i64>,
    target: String,
    pub(super) session: Option<PathBuf>,
    delivery_tracked: bool,
    submission_id: Option<String>,
    pub(super) delivered: bool,
}

impl RuntimeOwner {
    pub(super) fn retire_pending_prompt(&mut self) {
        if let Some(id) = self.pending_prompt_id.clone()
            && let Some(target) = self.pending_prompt_target.clone()
        {
            self.retired_prompts.insert(
                id,
                RetiredPrompt {
                    outbox_id: self.pending_outbox_id,
                    target,
                    session: self.active_session.clone(),
                    delivery_tracked: self.pending_prompt_delivery_tracked,
                    submission_id: self.pending_submission_id.clone(),
                    delivered: self.pending_prompt_result_emitted,
                },
            );
        }
    }

    pub(super) fn apply_cancelled_prompt(&mut self, event: &Value) -> bool {
        if event.get("type").and_then(Value::as_str) != Some("prompt_delivery")
            || event.get("status").and_then(Value::as_str) != Some("cancelled")
        {
            return false;
        }
        let Some(id) = event.get("submissionId").and_then(Value::as_str) else {
            return false;
        };
        let pending = self.pending_queued_prompts.remove(id);
        let retired = self.retired_prompts.remove(id);
        let current = self.pending_prompt_id.as_deref() == Some(id);
        let saved = (|| {
            let state = self.state.as_ref().ok_or("State unavailable")?;
            if let Some(outbox_id) = pending
                .as_ref()
                .map(|prompt| prompt.outbox_id)
                .or_else(|| retired.as_ref().and_then(|prompt| prompt.outbox_id))
                .or_else(|| current.then_some(self.pending_outbox_id).flatten())
            {
                state.with(|store| store.cancel_queued_prompts(&[outbox_id]))?;
            }
            if let Some(session) = self.active_session.as_deref() {
                state.with(|store| store.dismiss_prompt_receipt(session, id))?;
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = saved {
            zlog::error!("Queue cancellation was not saved: {error}");
        }
        if let Some(pending) = pending {
            self.emit_prompt_result(
                Some(&pending.submission_id),
                &pending.target,
                agents::PromptOutcome::Cancelled,
            );
        }
        if current {
            if let Some(target) = self.pending_prompt_target.take() {
                self.emit_prompt_result(
                    self.pending_submission_id.as_deref(),
                    &target,
                    agents::PromptOutcome::Cancelled,
                );
            }
            self.pending_prompt_id = None;
            self.pending_outbox_id = None;
            self.pending_submission_id = None;
            self.pending_prompt_result_emitted = false;
            self.pending_prompt_delivery_tracked = false;
            self.rollback_pending_prompt();
        }
        conversation_mut(self.active_snapshot_mut()).dismiss_pending_receipt(id);
        true
    }

    pub(super) fn apply_retired_prompt_delivery(
        &mut self,
        event: &Value,
    ) -> Option<SnapshotChange> {
        if event.get("type").and_then(Value::as_str) != Some("prompt_delivery") {
            return None;
        }
        let receipt_id = event.get("submissionId").and_then(Value::as_str)?;
        let retired = self.retired_prompts.get(receipt_id).cloned()?;
        let status = event
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if status == "delivered" {
            self.reconcile_retired_prompt(receipt_id, true);
            if retired.session == self.active_session && !retired.delivered {
                let mut message = event.get("message").cloned().unwrap_or_default();
                if !message.is_object() {
                    message = json!({});
                }
                message["queued"] = true.into();
                conversation_mut(self.active_snapshot_mut()).record_prompt_delivery(
                    receipt_id,
                    &message,
                    "delivered",
                );
                return Some(SnapshotChange::Immediate);
            }
        }
        Some(SnapshotChange::None)
    }

    pub(super) fn apply_prompt_delivery_receipt(&mut self, event: &Value) {
        if event.get("type").and_then(Value::as_str) != Some("prompt_delivery")
            || event.get("status").and_then(Value::as_str) != Some("delivered")
        {
            return;
        }
        let Some(receipt_id) = event.get("submissionId").and_then(Value::as_str) else {
            return;
        };
        let current = self.pending_prompt_id.as_deref() == Some(receipt_id);
        let queued = self.pending_queued_prompts.get(receipt_id).cloned();
        let outbox_id = if current {
            self.pending_outbox_id
        } else {
            queued.as_ref().map(|pending| pending.outbox_id)
        };
        let prompt = if current {
            self.pending_prompt_target.as_ref().map(|target| {
                (
                    target.clone(),
                    self.active_session.clone(),
                    self.pending_prompt_delivery_tracked,
                )
            })
        } else {
            queued.as_ref().map(|pending| {
                (
                    pending.target.clone(),
                    pending.session.clone(),
                    pending.delivery_tracked,
                )
            })
        };
        let mut saved = false;
        if let Some(state) = self.state.as_mut() {
            let result = state.with(|store| match (outbox_id, prompt) {
                (Some(outbox_id), Some((target, session, delivery_tracked))) => store
                    .complete_delivered_prompt(
                        outbox_id,
                        &target,
                        session.as_deref(),
                        receipt_id,
                        delivery_tracked,
                    ),
                _ => store.record_prompt_receipt_delivered(receipt_id, outbox_id),
            });
            match result {
                Ok(()) => saved = true,
                Err(error) => {
                    zlog::error!("Record prompt delivery receipt: {error}");
                }
            }
        }
        if current && saved {
            self.pending_outbox_id = None;
        }
        if current && !self.pending_prompt_result_emitted {
            if let (Some(submission_id), Some(target)) = (
                self.pending_submission_id.clone(),
                self.pending_prompt_target.clone(),
            ) {
                self.emit_prompt_result(
                    Some(&submission_id),
                    &target,
                    crate::agents::PromptOutcome::Accepted,
                );
            }
            // Recovered rows need the same delivery completion even when no
            // live composer submission exists to receive a result.
            self.pending_prompt_result_emitted = true;
            // Delivery completes the submission even when its admission reply
            // arrived earlier. A later duplicate reply no longer owns this slot.
            if saved {
                self.apply_response(crate::agents::SessionResponse::success(
                    Some(receipt_id.to_owned()),
                    crate::agents::SessionResponsePayload::Prompt(PromptMode::Normal),
                ));
            }
        } else if let Some(pending) = queued
            && !pending.result_emitted
        {
            self.emit_prompt_result(
                Some(&pending.submission_id),
                &pending.target,
                crate::agents::PromptOutcome::Accepted,
            );
            if let Some(pending) = self.pending_queued_prompts.get_mut(receipt_id) {
                pending.result_emitted = true;
            }
        }
    }

    /// A late receipt belongs to the old outbox row and exact composer submission,
    /// never a newer submission to the same target.
    pub(super) fn reconcile_retired_prompt(&mut self, id: &str, delivered: bool) -> bool {
        let Some(retired) = self.retired_prompts.get(id).cloned() else {
            return false;
        };
        let result = (|| {
            if let Some(state) = self.state.as_mut() {
                state.with(|store| {
                    if let Some(outbox_id) = retired.outbox_id {
                        if delivered {
                            store.complete_delivered_prompt(
                                outbox_id,
                                &retired.target,
                                retired.session.as_deref(),
                                id,
                                retired.delivery_tracked,
                            )?;
                        } else {
                            store.record_prompt_acceptance(
                                outbox_id,
                                &retired.target,
                                retired.session.as_deref(),
                                id,
                                retired.delivery_tracked,
                            )?;
                        }
                    } else if delivered && !retired.delivered {
                        store.record_prompt_receipt_delivered(id, None)?;
                    }
                    Ok(())
                })?;
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            zlog::error!("Save late prompt receipt: {error}");
            return true;
        }
        if self.active_session == retired.session && !retired.delivered {
            conversation_mut(self.active_snapshot_mut()).record_prompt_delivery(
                id,
                &json!({"queued":true}),
                if delivered { "delivered" } else { "accepted" },
            );
            if self.parked_snapshot.is_none() {
                self.publish();
            }
        }
        if delivered
            && !retired.delivered
            && let Some(submission_id) = &retired.submission_id
        {
            let _ = self.event_tx.send(RuntimeEvent::PromptResult {
                submission_id: Some(submission_id.clone()),
                target: retired.target.clone(),
                outcome: crate::agents::PromptOutcome::Accepted,
                session: retired.session.clone(),
            });
        }
        if let Some(retired) = self.retired_prompts.get_mut(id) {
            if delivered {
                retired.outbox_id = None;
            }
            retired.delivered |= delivered;
        }
        true
    }

    pub(super) fn release_pending_outbox(&mut self) {
        // The durable row stays pending. Runtime ownership ends here; the next
        // dispatch or restart can retry it.
        self.pending_outbox_id = None;
    }

    pub(super) fn fail_pending_queued_prompts(&mut self, _error: &str) {
        let pending = std::mem::take(&mut self.pending_queued_prompts);
        for (receipt_id, queued) in pending {
            if queued.result_emitted {
                if let Some(state) = self.state.as_mut()
                    && let Err(database_error) = state.with(|store| {
                        store.complete_delivered_prompt(
                            queued.outbox_id,
                            &queued.target,
                            queued.session.as_deref(),
                            &receipt_id,
                            queued.delivery_tracked,
                        )
                    })
                {
                    zlog::error!(
                        "Save proven queued delivery {}: {database_error}",
                        queued.outbox_id
                    );
                }
                continue;
            }
            // A reset before model admission is not a user-visible failure.
            // Keep the durable row pending and let the next process retry it.
            self.queued_prompts.push_back(crate::agents::QueuedPrompt {
                id: queued.outbox_id,
                submission_id: Some(queued.submission_id),
                target: queued.target,
                harness: queued.harness,
                project: queued.project,
                session: queued.session,
                mode: queued.mode,
                message: queued.message,
                display_message: queued.display_message,
                invocation: queued.invocation,
                images: queued.images,
            });
        }
    }

    pub(super) fn complete_current_delivered_prompt(&mut self) {
        if !self.pending_prompt_result_emitted {
            return;
        }
        let Some((outbox_id, receipt_id, target)) = self
            .pending_outbox_id
            .zip(self.pending_prompt_id.clone())
            .zip(self.pending_prompt_target.clone())
            .map(|((outbox_id, receipt_id), target)| (outbox_id, receipt_id, target))
        else {
            return;
        };
        let session = self.active_session.clone();
        let delivery_tracked = self.pending_prompt_delivery_tracked;
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match state.with(|store| {
            store.complete_delivered_prompt(
                outbox_id,
                &target,
                session.as_deref(),
                &receipt_id,
                delivery_tracked,
            )
        }) {
            Ok(()) => self.pending_outbox_id = None,
            Err(error) => {
                zlog::error!("Save proven prompt delivery {outbox_id}: {error}");
            }
        }
    }
}
