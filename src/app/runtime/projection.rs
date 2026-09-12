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
    let Some(context_window) = stats
        .pointer("/contextUsage/contextWindow")
        .and_then(Value::as_u64)
        .filter(|window| *window > 0)
    else {
        return false;
    };
    let percent = tokens as f64 * 100.0 / context_window as f64;
    if stats
        .pointer("/contextUsage/tokens")
        .and_then(Value::as_u64)
        == Some(tokens)
        && stats
            .pointer("/contextUsage/percent")
            .and_then(Value::as_f64)
            == Some(percent)
    {
        return false;
    }
    stats["contextUsage"]["tokens"] = tokens.into();
    stats["contextUsage"]["percent"] = percent.into();
    true
}

impl RuntimeOwner {
    pub(super) fn apply_response(&mut self, response: crate::agents::SessionResponse) {
        let operation = response.operation();
        let success = response.result.is_ok();
        if operation == SessionOperation::SelectModel {
            self.pending_session_controls.model_response(&response);
            if !success {
                if self.deferred_prompt.take().is_some() {
                    self.rollback_failed_prompt("The selected model could not be applied");
                    if let Some(target) = self.pending_prompt_target.take() {
                        self.emit_prompt_result(
                            &target,
                            crate::agents::PromptOutcome::RejectedBeforeAcceptance,
                        );
                    }
                }
                self.send(SessionCommand::LoadState);
            }
        }
        let is_prompt_response = matches!(operation, SessionOperation::Prompt(_))
            && response.id.as_ref() == self.pending_prompt_id.as_ref();
        if is_prompt_response {
            let outcome = match &response.result {
                Ok(_) => crate::agents::PromptOutcome::Accepted,
                Err(error)
                    if error.kind == crate::agents::SessionResponseErrorKind::DeliveryUnknown =>
                {
                    crate::agents::PromptOutcome::DeliveryUnknown
                }
                Err(_) => crate::agents::PromptOutcome::RejectedBeforeAcceptance,
            };
            if outcome != crate::agents::PromptOutcome::DeliveryUnknown {
                self.pending_prompt_id = None;
            }
            if outcome == crate::agents::PromptOutcome::RejectedBeforeAcceptance
                && operation == SessionOperation::Prompt(PromptMode::Normal)
            {
                self.normal_prompt_in_flight = false;
            }
            if outcome == crate::agents::PromptOutcome::Accepted {
                let target = self.pending_prompt_target.clone().unwrap_or_default();
                let session = self.active_session.clone();
                let delivery_tracked = self.pending_prompt_delivery_tracked;
                if let Some(id) = self.pending_outbox_id
                    && let Some(state) = self.state.as_mut()
                {
                    let receipt_id = response.id.as_deref().unwrap_or_default();
                    if let Err(error) = agents::complete_prompt_with_receipt(
                        state,
                        id,
                        &target,
                        session.as_deref(),
                        receipt_id,
                        delivery_tracked,
                    ) {
                        self.pending_prompt_item = None;
                        self.mark_outbox_delivery_unknown(&error);
                        self.pending_prompt_delivery_unknown = true;
                        self.pending_prompt_target.take();
                        self.emit_prompt_result(&target, crate::agents::PromptOutcome::Accepted);
                        self.fail(format!("Backend accepted the prompt, but saving its acknowledgement failed: {error}"));
                        return;
                    }
                    self.pending_outbox_id = None;
                }
            } else if let Err(error) = &response.result {
                self.invalidate_auto_title_generation();
                if outcome == crate::agents::PromptOutcome::DeliveryUnknown {
                    self.mark_outbox_delivery_unknown(&error.message);
                } else {
                    self.mark_outbox_failed(&error.message);
                }
            }
            match outcome {
                crate::agents::PromptOutcome::Accepted => {
                    if let Some(id) = response.id.as_deref() {
                        conversation_mut(self.active_snapshot_mut()).record_prompt_delivery(
                            id,
                            &serde_json::Value::Null,
                            "accepted",
                        );
                    }
                    self.pending_prompt_delivery_unknown = false;
                    self.pending_prompt_delivery_tracked = false;
                    self.pending_prompt_item = None;
                }
                crate::agents::PromptOutcome::RejectedBeforeAcceptance => {
                    self.pending_prompt_delivery_unknown = false;
                    self.pending_prompt_delivery_tracked = false;
                    self.rollback_pending_prompt();
                }
                crate::agents::PromptOutcome::DeliveryUnknown => {
                    if let (Some(id), Some(item)) =
                        (response.id.as_deref(), self.pending_prompt_item.take())
                    {
                        conversation_mut(self.active_snapshot_mut())
                            .bind_submitted_prompt(id, &item);
                        conversation_mut(self.active_snapshot_mut()).record_prompt_delivery(
                            id,
                            &serde_json::Value::Null,
                            "unknown",
                        );
                    }
                    self.pending_prompt_delivery_unknown = true;
                }
            }
            let target = if outcome == crate::agents::PromptOutcome::DeliveryUnknown {
                self.pending_prompt_target.clone()
            } else {
                self.pending_prompt_target.take()
            };
            if let Some(target) = target {
                self.emit_prompt_result(&target, outcome);
            }
        }
        if let Err(error) = &response.result {
            if is_prompt_response
                && error.kind == crate::agents::SessionResponseErrorKind::DeliveryUnknown
            {
                let running = self
                    .active_snapshot()
                    .session
                    .as_ref()
                    .is_some_and(|session| session.is_streaming);
                conversation_mut(self.active_snapshot_mut()).running = running;
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
                self.rollback_pending_prompt();
                self.deferred_prompt = None;
                if let Some(target) = self.pending_prompt_target.take() {
                    self.emit_prompt_result(
                        &target,
                        crate::agents::PromptOutcome::RejectedBeforeAcceptance,
                    );
                }
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
                snapshot.status = "Ready".into();
                self.startup_state_loaded = true;
                self.publish_session_metadata();
            }
            Payload::LoadHistory(history) => {
                if let SessionHistory::Replace(mut messages) = history {
                    if let (Some(state), Some(session)) =
                        (self.state.as_ref(), self.active_session.as_deref())
                    {
                        annotate_history_presentations(Some(state), session, &mut messages);
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

#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
