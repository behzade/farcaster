use super::*;

#[derive(Clone)]
pub(super) struct RetiredPrompt {
    pub(super) outbox_id: Option<i64>,
    pub(super) target: String,
    pub(super) session: Option<PathBuf>,
    pub(super) delivery_tracked: bool,
    pub(super) submission_id: Option<String>,
    pub(super) delivered: bool,
}

impl PendingQueuedPrompt {
    fn recovery_prompt(&self) -> agents::QueuedPrompt {
        agents::QueuedPrompt {
            id: self.outbox_id,
            submission_id: Some(self.submission_id.clone()),
            target: self.target.clone(),
            harness: self.harness,
            project: self.project.clone(),
            session: self.session.clone(),
            mode: self.mode,
            message: self.message.clone(),
            display_message: self.display_message.clone(),
            invocation: self.invocation.clone(),
            images: self.images.clone(),
        }
    }
}

impl RuntimeOwner {
    /// Successful admission waits for exact delivery. Terminal failure settles
    /// composer ownership once, regardless of which in-flight slot owns it.
    pub(super) fn settle_prompt_response(
        &mut self,
        response: &agents::SessionResponse,
        prompt: RetiredPrompt,
        recovery: Option<&PendingQueuedPrompt>,
    ) -> Option<agents::PromptOutcome> {
        use agents::{PromptOutcome, SessionResponseErrorKind};
        let receipt_id = response.id.as_deref()?;
        if !prompt.delivered && response.result.is_ok() {
            return None;
        }
        if prompt.delivered {
            if let Some(outbox_id) = prompt.outbox_id {
                let result = self
                    .state
                    .as_ref()
                    .ok_or("State unavailable".to_owned())
                    .and_then(|state| {
                        state.with(|store| {
                            store.complete_delivered_prompt(
                                outbox_id,
                                &prompt.target,
                                prompt.session.as_deref(),
                                receipt_id,
                                prompt.delivery_tracked,
                            )
                        })
                    });
                match result {
                    Ok(()) => {
                        if self.pending_prompt_id.as_deref() == Some(receipt_id) {
                            self.pending_outbox_id = None;
                        }
                        self.reconcile_saved_prompts();
                    }
                    Err(error) => {
                        zlog::error!("Save proven prompt delivery {outbox_id}: {error}");
                    }
                }
            }
            conversation_mut(self.active_snapshot_mut()).record_prompt_delivery(
                receipt_id,
                &Value::Null,
                "delivered",
            );
            return Some(PromptOutcome::Accepted);
        }
        let error = response
            .result
            .as_ref()
            .expect_err("terminal undelivered prompt");
        let uncertain = error.kind == SessionResponseErrorKind::DeliveryUnknown;
        let recover = uncertain
            || (error.kind == SessionResponseErrorKind::RejectedBeforeAcceptance
                && !is_user_actionable_prompt_error(&error.message));
        if uncertain {
            self.retired_prompts
                .insert(receipt_id.to_owned(), prompt.clone());
        }
        let outcome = self.settle_undelivered_outbox(
            prompt.outbox_id,
            prompt.submission_id.as_deref(),
            recover,
            recovery.map(PendingQueuedPrompt::recovery_prompt),
        );
        if !uncertain {
            // Terminal rejection owns no future model receipt. Dismiss only its
            // pending presentation, never a delivered transcript row.
            conversation_mut(self.active_snapshot_mut()).dismiss_pending_receipt(receipt_id);
        }
        self.emit_prompt_result(prompt.submission_id.as_deref(), &prompt.target, outcome);
        Some(outcome)
    }

    /// Restore composer ownership only after durable cancellation. Otherwise
    /// the saved queue owns the payload, including retries without a composer.
    pub(super) fn settle_undelivered_outbox(
        &mut self,
        outbox_id: Option<i64>,
        submission_id: Option<&str>,
        recover: bool,
        fallback: Option<agents::QueuedPrompt>,
    ) -> agents::PromptOutcome {
        use agents::PromptOutcome;
        let recover = recover || (submission_id.is_none() && outbox_id.is_some());
        let mut outcome = PromptOutcome::RejectedBeforeAcceptance;
        if let Some(outbox_id) = outbox_id {
            let cancelled = !recover
                && self.state.as_ref().is_some_and(|state| {
                    match state.with(|store| store.cancel_queued_prompts(&[outbox_id])) {
                        Ok(()) => true,
                        Err(error) => {
                            zlog::error!("Cancel rejected prompt {outbox_id}: {error}");
                            false
                        }
                    }
                });
            if cancelled {
                self.reconcile_saved_prompts();
            } else {
                self.save_outbox_for_recovery(outbox_id, submission_id, fallback);
                outcome = PromptOutcome::DeliveryUnknown;
            }
        } else if recover {
            outcome = PromptOutcome::DeliveryUnknown;
        }
        outcome
    }

    /// The durable outbox owns recovery membership; cached cards only project it.
    /// Call after delivery, cancellation, or native-history reconciliation.
    pub(super) fn reconcile_saved_prompts(&mut self) -> bool {
        if self.saved_prompts.is_empty() {
            return false;
        }
        let Some(state) = self.state.as_ref() else {
            return false;
        };
        match state.with(|store| store.queued_prompts()) {
            Ok(pending) => {
                let ids = pending
                    .into_iter()
                    .map(|prompt| prompt.id)
                    .collect::<HashSet<_>>();
                let previous_len = self.saved_prompts.len();
                self.saved_prompts.retain(|prompt| ids.contains(&prompt.id));
                previous_len != self.saved_prompts.len()
            }
            Err(error) => {
                zlog::error!("Reconcile saved prompts: {error}");
                false
            }
        }
    }

    pub(super) fn save_outbox_for_recovery(
        &mut self,
        id: i64,
        submission_id: Option<&str>,
        fallback: Option<agents::QueuedPrompt>,
    ) {
        if self.saved_prompts.iter().any(|prompt| prompt.id == id) {
            return;
        }
        let result = self
            .state
            .as_ref()
            .ok_or("State unavailable".to_owned())
            .and_then(|state| state.with(|store| store.queued_prompts()));
        match result {
            Ok(prompts) => {
                if let Some(mut prompt) = prompts.into_iter().find(|prompt| prompt.id == id) {
                    // Storage does not retain the live composer's submission identity.
                    prompt.submission_id = submission_id.map(str::to_owned);
                    self.saved_prompts.push_back(prompt);
                }
            }
            Err(error) => {
                zlog::error!("Load pending prompt {id}: {error}");
                // The secondary slot still owns the exact durable payload. A
                // failed read must not orphan it when composer ownership ends.
                if let Some(prompt) = fallback {
                    self.saved_prompts.push_back(prompt);
                }
            }
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
        } else {
            self.reconcile_saved_prompts();
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
        if saved {
            self.reconcile_saved_prompts();
            if current {
                self.pending_outbox_id = None;
            }
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
        let saved_changed = delivered && self.reconcile_saved_prompts();
        if saved_changed {
            self.publish();
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
        // Delivery may be uncertain. Keep the row visible until the user acts.
        if let Some(id) = self.pending_outbox_id.take() {
            self.park_pending_outbox(id);
        }
    }

    pub(super) fn park_pending_outbox(&mut self, id: i64) {
        if self.saved_prompts.iter().any(|prompt| prompt.id == id) {
            return;
        }
        let submission_id = self.pending_submission_id.clone();
        self.save_outbox_for_recovery(id, submission_id.as_deref(), None);
        if let Some(prompt) = self.saved_prompts.iter().find(|prompt| prompt.id == id) {
            self.emit_prompt_result(
                prompt.submission_id.as_deref(),
                &prompt.target,
                crate::agents::PromptOutcome::DeliveryUnknown,
            );
            self.publish();
        }
    }

    pub(super) fn fail_pending_queued_prompts(&mut self, error: &str) {
        let pending = std::mem::take(&mut self.pending_queued_prompts);
        for (receipt_id, queued) in pending {
            self.settle_prompt_response(
                &agents::SessionResponse::prompt_delivery_unknown(
                    receipt_id,
                    queued.mode,
                    error.to_owned(),
                ),
                RetiredPrompt {
                    outbox_id: Some(queued.outbox_id),
                    target: queued.target.clone(),
                    session: queued.session.clone(),
                    delivery_tracked: queued.delivery_tracked,
                    submission_id: Some(queued.submission_id.clone()),
                    delivered: queued.result_emitted,
                },
                Some(&queued),
            );
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

pub(super) fn is_user_actionable_prompt_error(message: &str) -> bool {
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
