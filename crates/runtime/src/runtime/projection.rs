use super::prompt_receipts::RetiredPrompt;
use super::*;
use crate::agents::{SessionHistory, SessionResponsePayload as Payload};

pub(super) fn stable_session_stats(previous: &Value, next: Value, running: bool) -> Value {
    if !running || context_usage_is_meaningful(&next) {
        return next;
    }
    let mut next = match next {
        Value::Object(next) => next,
        other => return other,
    };
    if let Some(context) = previous
        .get("contextUsage")
        .filter(|_| context_usage_is_meaningful(previous))
    {
        next.insert("contextUsage".into(), context.clone());
    } else {
        next.remove("contextUsage");
    }
    Value::Object(next)
}

fn context_usage_is_meaningful(stats: &Value) -> bool {
    let Some(context) = stats.get("contextUsage") else {
        return false;
    };
    context
        .get("tokens")
        .and_then(Value::as_u64)
        .is_some_and(|tokens| tokens > 0)
        || context
            .get("percent")
            .and_then(Value::as_f64)
            .is_some_and(|percent| percent.is_finite() && percent > 0.0)
}

pub(super) fn historical_context_stats(messages: &[Value], models: &[Model]) -> Value {
    let Some(message) = messages.iter().rev().find(|message| {
        message.get("role").and_then(Value::as_str) == Some("assistant")
            && !matches!(
                message.get("stopReason").and_then(Value::as_str),
                Some("aborted" | "error")
            )
    }) else {
        return Value::Null;
    };
    let Some(usage) = message.get("usage") else {
        return Value::Null;
    };
    let tokens = usage
        .get("totalTokens")
        .and_then(Value::as_u64)
        .filter(|tokens| *tokens > 0)
        .or_else(|| {
            ["input", "output", "cacheRead", "cacheWrite"]
                .iter()
                .try_fold(0_u64, |total, key| {
                    total.checked_add(usage.get(*key)?.as_u64()?)
                })
                .filter(|tokens| *tokens > 0)
        });
    let Some(tokens) = tokens else {
        return Value::Null;
    };
    let provider = message.get("provider").and_then(Value::as_str);
    let model_id = message.get("model").and_then(Value::as_str);
    let context_window = models
        .iter()
        .find(|model| {
            Some(model.provider.as_str()) == provider && Some(model.id.as_str()) == model_id
        })
        .map(|model| model.context_window)
        .filter(|window| *window > 0);
    let mut context = json!({"tokens": tokens});
    if let Some(context_window) = context_window {
        context["contextWindow"] = context_window.into();
        context["percent"] = (tokens as f64 * 100.0 / context_window as f64).into();
    }
    json!({"contextUsage": context})
}

pub(super) fn update_context_from_event(stats: &mut Value, event: &Value) -> bool {
    let usage = event
        .get("usage")
        .or_else(|| event.pointer("/message/usage"));
    let Some(usage) = usage else { return false };
    let tokens = usage
        .get("totalTokens")
        .and_then(Value::as_u64)
        .filter(|tokens| *tokens > 0)
        .or_else(|| {
            ["input", "output", "cacheRead", "cacheWrite"]
                .iter()
                .try_fold(0_u64, |total, key| {
                    total.checked_add(usage.get(*key)?.as_u64()?)
                })
                .filter(|tokens| *tokens > 0)
        });
    let Some(tokens) = tokens else { return false };
    let current_window = stats
        .pointer("/contextUsage/contextWindow")
        .and_then(Value::as_u64)
        .filter(|window| *window > 0);
    let Some(context_window) = event
        .get("contextWindow")
        .and_then(Value::as_u64)
        .filter(|window| *window > 0)
        .or(current_window)
    else {
        return false;
    };
    let percent = tokens as f64 * 100.0 / context_window as f64;
    if stats
        .pointer("/contextUsage/tokens")
        .and_then(Value::as_u64)
        == Some(tokens)
        && current_window == Some(context_window)
        && stats
            .pointer("/contextUsage/percent")
            .and_then(Value::as_f64)
            == Some(percent)
    {
        return false;
    }
    stats["contextUsage"]["tokens"] = tokens.into();
    stats["contextUsage"]["contextWindow"] = context_window.into();
    stats["contextUsage"]["percent"] = percent.into();
    true
}

pub(super) fn update_tokens_from_event(stats: &mut Value, event: &Value) -> bool {
    let Some(tokens) = event.get("sessionUsage").filter(|tokens| {
        ["input", "output", "cacheRead", "cacheWrite", "totalTokens"]
            .iter()
            .all(|key| tokens.get(*key).and_then(Value::as_u64).is_some())
    }) else {
        return false;
    };
    if stats.get("tokens") == Some(tokens) {
        return false;
    }
    stats["tokens"] = tokens.clone();
    true
}

impl RuntimeOwner {
    pub(super) fn apply_response(&mut self, response: crate::agents::SessionResponse) {
        if matches!(response.operation(), SessionOperation::Prompt(_))
            && let Some(id) = response.id.as_deref()
            && self.retired_prompts.contains_key(id)
        {
            if response.result.is_ok() {
                self.reconcile_retired_prompt(id, false);
            }
            return;
        }
        let operation = response.operation();
        let success = response.result.is_ok();
        if matches!(operation, SessionOperation::Prompt(_))
            && response.result.as_ref().is_err_and(|error| {
                error.kind == crate::agents::SessionResponseErrorKind::Cancelled
            })
            && response.id != self.pending_prompt_id
            && !response
                .id
                .as_ref()
                .is_some_and(|id| self.pending_queued_prompts.contains_key(id))
        {
            // Exact cancellation receipts have already removed their pending
            // submission. Their terminal reply must not restore a draft or add
            // a spurious command-failed row.
            return;
        }
        if matches!(operation, SessionOperation::Prompt(_))
            && let Some(request_id) = response.id.as_deref()
            && let Some(pending) = self.pending_queued_prompts.remove(request_id)
        {
            let outcome = self.settle_prompt_response(
                &response,
                RetiredPrompt {
                    outbox_id: Some(pending.outbox_id),
                    target: pending.target.clone(),
                    session: pending.session.clone(),
                    delivery_tracked: pending.delivery_tracked,
                    submission_id: Some(pending.submission_id.clone()),
                    delivered: pending.result_emitted,
                },
                Some(&pending),
            );
            if outcome.is_none() {
                self.pending_queued_prompts
                    .insert(request_id.to_owned(), pending);
            }
            if self.parked_snapshot.is_none() {
                self.publish();
            }
            return;
        }
        if operation == SessionOperation::SelectModel {
            self.pending_session_controls.model_response(&response);
            if !success && !self.pending_session_controls.model_pending() {
                self.active_snapshot_mut().pending_initial_model = false;
                self.settle_deferred_prompt();
                self.send(SessionCommand::LoadState);
            }
        }
        if operation == SessionOperation::SelectServiceTier {
            self.pending_session_controls.tier_response(&response);
            if !success && !self.pending_session_controls.service_tier_pending() {
                self.active_snapshot_mut().pending_initial_service_tier = false;
                self.settle_deferred_prompt();
                self.send(SessionCommand::LoadState);
            }
        }
        if operation == SessionOperation::SelectReasoning {
            let selected = self.pending_session_controls.thinking_response(&response);
            if let Some(level) = selected
                && let Some(state) = self.active_snapshot_mut().session.as_mut()
            {
                state.thinking_level = level;
            }
            if !success && !self.pending_session_controls.thinking_pending() {
                self.settle_deferred_prompt();
                self.send(SessionCommand::LoadState);
            }
        }
        let is_prompt_response = matches!(operation, SessionOperation::Prompt(_))
            && response.id.as_ref() == self.pending_prompt_id.as_ref();
        let prompt_was_delivered = is_prompt_response && self.pending_prompt_result_emitted;
        if is_prompt_response {
            let Some(outcome) = self.settle_prompt_response(
                &response,
                RetiredPrompt {
                    outbox_id: self.pending_outbox_id,
                    target: self.pending_prompt_target.clone().unwrap_or_default(),
                    session: self.active_session.clone(),
                    delivery_tracked: self.pending_prompt_delivery_tracked,
                    submission_id: self.pending_submission_id.clone(),
                    delivered: prompt_was_delivered,
                },
                None,
            ) else {
                return;
            };
            if outcome != crate::agents::PromptOutcome::Accepted {
                self.normal_prompt_in_flight = false;
                self.invalidate_auto_title_generation();
                self.rollback_pending_prompt();
                self.pending_outbox_id = None;
            } else {
                self.pending_prompt_item = None;
            }
            self.pending_prompt_id = None;
            self.pending_prompt_target = None;
            self.pending_prompt_delivery_tracked = false;
            self.pending_submission_id = None;
            self.pending_prompt_result_emitted = false;
            if outcome == crate::agents::PromptOutcome::DeliveryUnknown {
                let running = self
                    .active_snapshot()
                    .session
                    .as_ref()
                    .is_some_and(|session| session.is_streaming);
                conversation_mut(self.active_snapshot_mut()).running = running;
                if !running {
                    self.active_snapshot_mut().status = "Done".into();
                }
                if self.parked_snapshot.is_none() {
                    self.publish();
                }
                return;
            }
        }
        if prompt_was_delivered && response.result.is_err() {
            if self.parked_snapshot.is_none() {
                self.publish();
            }
            return;
        }
        if let Err(error) = &response.result {
            if is_prompt_response
                && error.kind == crate::agents::SessionResponseErrorKind::Cancelled
            {
                // Abort deliberately returns ownership of an undelivered input
                // to the composer. It is not a command failure and should not
                // add an error row after the prompt result restores the draft.
                let running = self
                    .active_snapshot()
                    .session
                    .as_ref()
                    .is_some_and(|session| session.is_streaming);
                conversation_mut(self.active_snapshot_mut()).running = running;
                if !running {
                    self.active_snapshot_mut().status = "Stopped".into();
                }
                self.maybe_send_deferred_prompt();
                if self.parked_snapshot.is_none() {
                    self.publish();
                }
                return;
            }
            let startup_query = matches!(
                operation,
                SessionOperation::LoadState | SessionOperation::LoadHistory
            );
            let blocks_resume = self.deferred_prompt.is_some() && startup_query;
            let blocks_session_command_resume =
                !self.pending_session_controls.is_empty() && startup_query;
            // Ignore cancelled refreshes unless they gate a pending prompt or control.
            if error.kind == crate::agents::SessionResponseErrorKind::Cancelled
                && (startup_query || operation == SessionOperation::LoadUsage)
                && !blocks_resume
                && !blocks_session_command_resume
            {
                return;
            }
            if blocks_session_command_resume {
                let details = format!("{operation:?}: {}", error.message);
                self.fail_session_control_resume("Command not sent", "Command not sent", details);
                return;
            }
            let snapshot = self.active_snapshot_mut();
            conversation_mut(snapshot).push_local_error(
                "Command failed",
                format!("{operation:?}: {}", error.message),
            );
            snapshot.status = "Command failed".into();
            if blocks_resume {
                self.settle_deferred_prompt();
                if let Some(snapshot) = self.parked_snapshot.take() {
                    self.snapshot = snapshot;
                }
            }
            if is_prompt_response {
                let running = self
                    .active_snapshot()
                    .session
                    .as_ref()
                    .is_some_and(|session| session.is_streaming);
                conversation_mut(self.active_snapshot_mut()).running = running;
                self.maybe_send_deferred_prompt();
            }
            if self.parked_snapshot.is_none() {
                self.publish();
            }
            return;
        }
        let Ok(payload) = response.result else { return };
        match payload {
            Payload::LoadState(state) => {
                let model_change_sent = !self.pending_session_controls.model_pending();
                let tier_change_sent = !self.pending_session_controls.service_tier_pending();
                let normal_prompt_in_flight = self.normal_prompt_in_flight;
                let selected_session = state
                    .session_file
                    .as_ref()
                    .map(PathBuf::from)
                    .map(|path| crate::sessions::normalize_session_path(&path))
                    .or_else(|| self.active_session.clone());
                self.active_session = selected_session.clone();
                let snapshot = self.active_snapshot_mut();
                snapshot.selected_session = selected_session;
                conversation_mut(snapshot).running = state.is_streaming || normal_prompt_in_flight;
                snapshot.session = Some(*state);
                let model_confirmed = snapshot
                    .session
                    .as_ref()
                    .and_then(|session| session.model.as_ref())
                    .zip(snapshot.prefill_model.as_ref())
                    .is_some_and(|(current, requested)| {
                        let current = snapshot.catalog_model(current);
                        current.provider == requested.provider && current.id == requested.id
                    });
                if snapshot.pending_initial_model && model_change_sent && model_confirmed {
                    snapshot.pending_initial_model = false;
                }
                if snapshot.pending_initial_service_tier
                    && tier_change_sent
                    && snapshot
                        .session
                        .as_ref()
                        .and_then(|session| session.service_tier.as_ref())
                        == snapshot.prefill_service_tier.as_ref()
                {
                    snapshot.pending_initial_service_tier = false;
                }
                snapshot.status = "Ready".into();
                self.startup_state_loaded = true;
                self.publish_session_metadata();
            }
            Payload::LoadHistory(history) => {
                if let SessionHistory::Replace {
                    mut messages,
                    prompt_deliveries,
                } = history
                {
                    if let (Some(state), Some(session), Some(evidence)) = (
                        self.state.as_mut(),
                        self.active_session.as_deref(),
                        prompt_deliveries.as_ref(),
                    ) && let Err(error) =
                        state.with(|store| store.reconcile_prompt_deliveries(session, evidence))
                    {
                        zlog::error!("Reconcile saved prompt deliveries: {error}");
                    }
                    self.reconcile_saved_prompts();
                    if let (Some(state), Some(session)) =
                        (self.state.as_ref(), self.active_session.as_deref())
                    {
                        if let Err(error) = state.with(|store| {
                            annotate_history_presentations(Some(store), session, &mut messages);
                            Ok(())
                        }) {
                            zlog::error!("Annotate prompt deliveries: {error}");
                        }
                    }
                    conversation_mut(self.active_snapshot_mut()).replace_history(&messages);
                    // Deferred delivery restores the local row after both startup responses.
                    self.pending_prompt_item = None;
                }
                self.startup_history_loaded = true;
                self.publish_session_metadata();
            }
            Payload::ListModels(models) => self.active_snapshot_mut().models = models,
            Payload::ListReasoningLevels(levels) => {
                self.active_snapshot_mut().thinking_levels = levels
            }
            Payload::ListModes { modes, selected } => {
                let snapshot = self.active_snapshot_mut();
                snapshot.modes = modes;
                snapshot.selected_mode = selected;
            }
            Payload::LoadUsage(usage) => {
                let running = self.active_snapshot().conversation.running;
                let previous = self.active_snapshot().stats.clone();
                // The transcript/activity projection still uses a JSON stats document.
                // Response validation has already happened at the adapter boundary.
                self.active_snapshot_mut().stats =
                    stable_session_stats(&previous, json!(usage), running);
                self.publish_session_metadata();
            }
            Payload::ListCommands(commands) => self.active_snapshot_mut().commands = commands,
            Payload::SelectModel(model) => {
                if let Some(state) = self.active_snapshot_mut().session.as_mut() {
                    state.model = Some(model);
                }
                self.send(SessionCommand::ListReasoningLevels);
                self.send(SessionCommand::LoadState);
                self.send(SessionCommand::LoadUsage);
            }
            Payload::SelectReasoning | Payload::SelectServiceTier => {
                self.send(SessionCommand::LoadState);
            }
            Payload::SelectMode => {
                self.send(SessionCommand::ListModes);
                self.send(SessionCommand::LoadState);
            }
            Payload::Prompt(_) => {
                self.active_snapshot_mut().status = "Accepted".into();
                self.send(SessionCommand::LoadState);
            }
            Payload::Abort => self.active_snapshot_mut().status = "Stopping".into(),
            Payload::Compact => self.send(SessionCommand::LoadState),
            Payload::Rename => {
                self.active_snapshot_mut().status = "Session named".into();
                self.send(SessionCommand::LoadState);
                self.publish_session_metadata();
            }
            Payload::ExportHtml { path } => {
                self.active_snapshot_mut().status = format!("Exported to {path}");
            }
            _ => {}
        }
        if matches!(
            operation,
            SessionOperation::LoadState | SessionOperation::LoadHistory
        ) {
            self.maybe_send_pending_session_controls();
            self.maybe_send_deferred_prompt();
        }
        if self.parked_snapshot.is_none() {
            self.publish();
        }
    }
}

pub(super) fn update_session_goal_from_event(
    goal: &mut Option<crate::agents::SessionGoal>,
    kind: &SessionActivityKind,
    event: &Value,
) -> bool {
    if kind != &SessionActivityKind::SessionGoalChanged {
        return false;
    }
    let Some(value) = event.get("goal") else {
        return false;
    };
    let Ok(updated) = serde_json::from_value::<Option<crate::agents::SessionGoal>>(value.clone())
    else {
        return false;
    };
    if *goal == updated {
        return false;
    }
    *goal = updated;
    true
}

pub(super) fn update_account_usage_from_event(
    usage: &mut crate::agents::AccountUsage,
    kind: &SessionActivityKind,
    event: &Value,
) -> bool {
    if kind != &SessionActivityKind::AccountUsageChanged {
        return false;
    }
    let Some(value) = event.get("usage") else {
        return false;
    };
    let Ok(updated) = serde_json::from_value::<crate::agents::AccountUsage>(value.clone()) else {
        return false;
    };
    if *usage == updated {
        return false;
    }
    *usage = updated;
    true
}

#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
