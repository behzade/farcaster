use super::*;

pub(super) struct BatchInput {
    pub(super) delivery: NativeInputDelivery,
    pub(super) needs_ack: bool,
    // Only claimed handoff input can be retried after definite rejection.
    pub(super) claimed: Option<(String, Vec<CodexUserInput>)>,
}

impl BatchInput {
    pub(super) fn finish_request(&mut self) -> Option<String> {
        self.claimed = None;
        std::mem::take(&mut self.needs_ack)
            .then(|| self.delivery.submission_id.clone())
            .flatten()
    }
}

impl CodexWorkerSession {
    pub(super) fn capture_handoff_target(&mut self, turn_id: &str) {
        if let Some(handoff) = self.handoff.as_mut()
            && handoff.phase == HandoffPhase::Interrupting
            && handoff.target_turn.is_none()
        {
            handoff.target_turn = Some(turn_id.to_owned());
        }
    }

    pub(super) fn control_response(
        &mut self,
        operation: &'static str,
        client_id: &str,
        result: &Value,
    ) -> Result<(), String> {
        if operation == "steer" {
            if result.get("turnId").and_then(Value::as_str).is_none() {
                return Err("decode Codex steer acknowledgement: missing turnId".into());
            }
            return self.maybe_submit_handoff();
        }
        if operation != "queue" {
            return Ok(());
        }
        let queue_id = queue_submission_id(result)?;
        let should_delete = if let Some(input) = self.native_inputs.get_mut(client_id) {
            let NativeInputKind::Queue {
                queue_id: stored_id,
                claim_pending,
                ..
            } = &mut input.kind
            else {
                return Ok(());
            };
            *stored_id = Some(queue_id.clone());
            let should_delete = input.handoff
                && self.handoff.as_ref().is_some_and(|handoff| {
                    handoff.cancelled || handoff.phase == HandoffPhase::Claiming
                });
            if should_delete {
                *claim_pending = true;
            }
            should_delete
        } else {
            false
        };
        if should_delete {
            self.delete_queued_input(client_id, &queue_id)?;
        }
        Ok(())
    }

    pub(super) fn begin_handoff_claims(&mut self) -> Result<(), String> {
        let Some(handoff) = self.handoff.as_mut() else {
            return Ok(());
        };
        handoff.phase = HandoffPhase::Claiming;
        let mut deletes = Vec::new();
        for client_id in &self.native_input_order {
            let Some(input) = self.native_inputs.get_mut(client_id) else {
                continue;
            };
            if !input.handoff {
                continue;
            }
            if let NativeInputKind::Queue {
                queue_id: Some(queue_id),
                claim_pending,
                claimed,
                ..
            } = &mut input.kind
                && !*claim_pending
                && !*claimed
            {
                *claim_pending = true;
                deletes.push((client_id.clone(), queue_id.clone()));
            }
        }
        for (client_id, queue_id) in deletes {
            self.delete_queued_input(&client_id, &queue_id)?;
        }
        self.maybe_submit_handoff()
    }

    fn delete_queued_input(&mut self, client_id: &str, queue_id: &str) -> Result<(), String> {
        let id = self.request(
            "thread/queue/delete",
            json!({"threadId": self.thread_id, "queuedSubmissionId": queue_id}),
        )?;
        self.pending.insert(
            id,
            PendingRequest::QueueDelete {
                client_id: client_id.to_owned(),
            },
        );
        Ok(())
    }

    pub(super) fn queue_delete_response(
        &mut self,
        client_id: &str,
        deleted: Option<bool>,
    ) -> Result<(), String> {
        let Some(input) = self.native_inputs.get_mut(client_id) else {
            return Ok(());
        };
        let NativeInputKind::Queue {
            claim_pending,
            claimed,
            claim_lost,
            ..
        } = &mut input.kind
        else {
            return Ok(());
        };
        *claim_pending = false;
        let cancelled = self
            .handoff
            .as_ref()
            .is_some_and(|handoff| handoff.cancelled);
        match deleted {
            Some(true) => *claimed = true,
            Some(false) => {
                *claim_lost = true;
                if let Some(handoff) = self.handoff.as_mut() {
                    handoff.wait_for_active_turn = true;
                    input.cancel_on_delivery = handoff.cancelled;
                }
            }
            None => {
                input.handoff = false;
                return Err("decode Codex queue deletion: missing deleted flag".into());
            }
        }
        if deleted == Some(true) && cancelled {
            self.native_inputs.remove(client_id);
            self.native_input_order.retain(|queued| queued != client_id);
            self.client_submissions.remove(client_id);
            self.finish_cancelled_handoff();
        }
        self.maybe_submit_handoff()
    }

    pub(super) fn maybe_submit_handoff(&mut self) -> Result<(), String> {
        let Some(handoff) = self.handoff.as_ref() else {
            return Ok(());
        };
        if handoff.cancelled || handoff.phase != HandoffPhase::Claiming {
            return Ok(());
        }
        if handoff.wait_for_active_turn && self.current_turn.is_none() {
            return Ok(());
        }
        let mut selected = Vec::new();
        for client_id in &self.native_input_order {
            let Some(input) = self.native_inputs.get(client_id) else {
                continue;
            };
            if !input.handoff {
                continue;
            }
            match &input.kind {
                NativeInputKind::Steer {
                    receipt: SteerReceipt::Pending,
                } => return Ok(()),
                NativeInputKind::Steer {
                    receipt: SteerReceipt::Accepted | SteerReceipt::RejectedByTurnRace,
                } => selected.push(client_id.clone()),
                NativeInputKind::Unknown => {}
                NativeInputKind::Retry => selected.push(client_id.clone()),
                NativeInputKind::Queue { queue_id: None, .. }
                | NativeInputKind::Queue {
                    claim_pending: true,
                    ..
                } => return Ok(()),
                NativeInputKind::Queue { claimed: true, .. } => selected.push(client_id.clone()),
                NativeInputKind::Queue { claimed: false, .. } => {}
            }
        }
        if selected.is_empty() {
            self.handoff = None;
            return Ok(());
        }

        let batch_client_id = format!(
            "{HANDOFF_CLIENT_ID_PREFIX}{}",
            self.next_id.saturating_add(1)
        );
        let mut batch_input = Vec::new();
        for client_id in &selected {
            let input = self
                .native_inputs
                .get(client_id)
                .expect("selected native input must still exist");
            if !batch_input.is_empty() {
                batch_input.push(CodexUserInput::text("\n\n"));
            }
            batch_input.extend(input.input.clone());
        }
        let active_turn = self.current_turn.clone();
        let (method, params, starts_turn) = if let Some(turn_id) = active_turn {
            (
                "turn/steer",
                json!({
                    "threadId": self.thread_id,
                    "expectedTurnId": turn_id,
                    "clientUserMessageId": batch_client_id,
                    "input": batch_input,
                }),
                false,
            )
        } else {
            (
                "turn/start",
                json!({
                    "threadId": self.thread_id,
                    "clientUserMessageId": batch_client_id,
                    "input": batch_input,
                    "model": self.model,
                    "effort": self.effort,
                    "collaborationMode": self.collaboration_mode,
                }),
                true,
            )
        };
        let id = self.submission_request(method, params)?;
        let mut deliveries = Vec::new();
        for client_id in selected {
            let input = self
                .native_inputs
                .remove(&client_id)
                .expect("selected native input must still exist");
            let needs_ack = input
                .delivery
                .submission_id
                .as_ref()
                .is_some_and(|id| !self.acknowledged_prompts.contains(id));
            deliveries.push(BatchInput {
                delivery: input.delivery,
                needs_ack,
                claimed: Some((client_id.clone(), input.input)),
            });
            self.client_submissions.remove(&client_id);
            self.native_input_order
                .retain(|queued| queued != &client_id);
        }
        self.batch_deliveries
            .insert(batch_client_id.clone(), deliveries);
        if let Some(handoff) = self.handoff.as_mut() {
            handoff.phase = HandoffPhase::Submitted;
            handoff.batch_client_id = Some(batch_client_id.clone());
        }
        self.pending.insert(
            id,
            PendingRequest::HandoffTurn {
                client_id: batch_client_id,
                starts_turn,
            },
        );
        if starts_turn {
            self.caller_identity
                .set_activity(WorkerActivityState::Starting);
        }
        Ok(())
    }

    pub(super) fn cancel_handoff(&mut self) -> Result<(), String> {
        let Some(handoff) = self.handoff.as_mut() else {
            return Ok(());
        };
        handoff.cancelled = true;
        let mut deletes = Vec::new();
        for (client_id, input) in &mut self.native_inputs {
            if !input.handoff {
                continue;
            }
            if let NativeInputKind::Queue {
                queue_id: Some(queue_id),
                claim_pending,
                claimed,
                claim_lost,
            } = &mut input.kind
            {
                if *claim_lost {
                    input.cancel_on_delivery = true;
                } else if !*claim_pending && !*claimed {
                    *claim_pending = true;
                    deletes.push((client_id.clone(), queue_id.clone()));
                }
            }
        }
        for (client_id, queue_id) in deletes {
            self.delete_queued_input(&client_id, &queue_id)?;
        }
        Ok(())
    }

    pub(super) fn discard_cancelled_steers(&mut self) {
        if !self
            .handoff
            .as_ref()
            .is_some_and(|handoff| handoff.cancelled)
        {
            return;
        }
        let discarded = self
            .native_inputs
            .iter()
            .filter(|(_, input)| {
                input.handoff && matches!(input.kind, NativeInputKind::Steer { .. })
            })
            .map(|(client_id, _)| client_id.clone())
            .collect::<Vec<_>>();
        for client_id in discarded {
            self.native_inputs.remove(&client_id);
            self.native_input_order
                .retain(|queued| queued != &client_id);
        }
        self.finish_cancelled_handoff();
    }

    pub(super) fn finish_cancelled_handoff(&mut self) {
        let Some(handoff) = self.handoff.as_ref().filter(|handoff| handoff.cancelled) else {
            return;
        };
        let current_batch = handoff.batch_client_id.clone();
        let has_originals = self.native_inputs.values().any(|input| input.handoff);
        let has_current_batch = current_batch
            .as_ref()
            .is_some_and(|client_id| self.batch_deliveries.contains_key(client_id));
        if !has_originals && !has_current_batch {
            self.handoff = None;
        }
    }

    pub(super) fn reject_handoff(&mut self, client_id: &str, error: &str) -> bool {
        let cancelled = self.handoff.as_ref().is_some_and(|handoff| {
            handoff.batch_client_id.as_deref() == Some(client_id) && handoff.cancelled
        });
        let mut retry = VecDeque::new();
        if let Some(batch) = self.batch_deliveries.remove(client_id) {
            for entry in batch {
                if entry.needs_ack {
                    if let Some(id) = entry.delivery.submission_id {
                        self.record_prompt_ack(id, Err(error.into()));
                    }
                } else if !cancelled && let Some((id, input)) = entry.claimed {
                    // Admission preceded the handoff, so the caller no longer
                    // owns this input. Preserve it for explicit retry only.
                    self.native_inputs.insert(
                        id.clone(),
                        PendingNativeInput {
                            input,
                            delivery: entry.delivery,
                            kind: NativeInputKind::Retry,
                            handoff: false,
                            cancel_on_delivery: false,
                        },
                    );
                    retry.push_back(id);
                }
            }
        }
        let retained = !retry.is_empty();
        retry.append(&mut self.native_input_order);
        self.native_input_order = retry;
        if self
            .handoff
            .as_ref()
            .is_some_and(|handoff| handoff.batch_client_id.as_deref() == Some(client_id))
        {
            self.handoff = None;
        }
        retained
    }

    pub(super) fn discard_retry_inputs(&mut self) {
        self.native_inputs
            .retain(|_, input| !matches!(input.kind, NativeInputKind::Retry));
        self.native_input_order
            .retain(|id| self.native_inputs.contains_key(id));
    }
}
