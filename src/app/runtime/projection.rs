use super::*;

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
        let operation = response.operation;
        let is_prompt_response = matches!(operation, SessionOperation::Prompt(_))
            && response.id.as_ref() == self.pending_prompt_id.as_ref();
        if is_prompt_response {
            self.pending_prompt_id = None;
            if response.success {
                let target = self.pending_prompt_target.clone().unwrap_or_default();
                let session = self.active_session.clone();
                if let Some(id) = self.pending_outbox_id.take()
                    && let Some(state) = self.state.as_mut()
                {
                    let _ = agents::complete_prompt(state, id, &target, session.as_deref());
                }
            } else {
                self.invalidate_auto_title_generation();
                self.mark_outbox_failed(
                    response
                        .error
                        .as_deref()
                        .unwrap_or("The harness rejected the prompt"),
                );
            }
            if response.success {
                self.pending_prompt_item = None;
            } else {
                self.rollback_pending_prompt();
            }
            if let Some(target) = self.pending_prompt_target.take() {
                self.emit_prompt_result(&target, response.success);
            }
        }
        if !response.success {
            let startup_query = matches!(
                operation,
                SessionOperation::LoadState | SessionOperation::LoadHistory
            );
            let blocks_resume = self.deferred_prompt.is_some() && startup_query;
            let blocks_session_command_resume =
                !self.pending_session_controls.is_empty() && startup_query;
            if blocks_session_command_resume {
                let details = format!(
                    "{operation:?}: {}",
                    response.error.unwrap_or_else(|| "command failed".into())
                );
                self.fail_session_control_resume("Command not sent", "Command not sent", details);
                return;
            }
            let snapshot = self.active_snapshot_mut();
            conversation_mut(snapshot).push_local_error(
                "Command failed",
                format!(
                    "{operation:?}: {}",
                    response.error.unwrap_or_else(|| "command failed".into())
                ),
            );
            snapshot.status = "Command failed".into();
            if blocks_resume {
                self.rollback_pending_prompt();
                self.deferred_prompt = None;
                if let Some(target) = self.pending_prompt_target.take() {
                    self.emit_prompt_result(&target, false);
                }
                if let Some(snapshot) = self.parked_snapshot.take() {
                    self.snapshot = snapshot;
                }
            }
            if self.parked_snapshot.is_none() {
                self.publish();
            }
            return;
        }
        match operation {
            SessionOperation::LoadState => {
                match serde_json::from_value::<SessionState>(response.data) {
                    Ok(state) => {
                        let previous_session = self.active_session.clone();
                        let selected_session = state
                            .session_file
                            .as_ref()
                            .map(PathBuf::from)
                            .map(|path| crate::sessions::normalize_session_path(&path))
                            .or_else(|| self.active_session.clone());
                        self.active_session = selected_session.clone();
                        let snapshot = self.active_snapshot_mut();
                        snapshot.selected_session = selected_session;
                        conversation_mut(snapshot).running = state.is_streaming;
                        snapshot.session = Some(state);
                        snapshot.status = "Ready".into();
                        self.startup_state_loaded = true;
                        if self.active_session.is_some() && self.active_session != previous_session
                        {
                            self.refresh_sessions();
                        }
                    }
                    Err(error) => {
                        self.fail(format!("decode get_state: {error}"));
                        return;
                    }
                }
            }
            SessionOperation::LoadHistory => {
                if response.data.get("preserve").and_then(Value::as_bool) != Some(true) {
                    let entries = response
                        .data
                        .get("entries")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let mut messages = project_display_history(&entries);
                    if let (Some(state), Some(session)) =
                        (self.state.as_ref(), self.active_session.as_deref())
                    {
                        annotate_history_presentations(Some(state), session, &mut messages);
                    }
                    conversation_mut(self.active_snapshot_mut()).replace_history(&messages);
                }
                self.startup_history_loaded = true;
            }
            SessionOperation::ListModels => {
                self.active_snapshot_mut().models = response
                    .data
                    .get("models")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default();
            }
            SessionOperation::ListReasoningLevels => {
                self.active_snapshot_mut().thinking_levels = response
                    .data
                    .get("levels")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
            }
            SessionOperation::ListModes => {
                self.active_snapshot_mut().modes = response
                    .data
                    .get("modes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|mode| serde_json::from_value(mode.clone()).ok())
                    .collect();
                self.active_snapshot_mut().selected_mode = response
                    .data
                    .get("selected")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            SessionOperation::LoadUsage => {
                let running = self.active_snapshot().conversation.running;
                let previous = self.active_snapshot().stats.clone();
                self.active_snapshot_mut().stats =
                    stable_session_stats(&previous, response.data, running);
            }
            SessionOperation::ListCommands => {
                self.active_snapshot_mut().commands = response
                    .data
                    .get("commands")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|command| serde_json::from_value(command.clone()).ok())
                    .collect()
            }
            SessionOperation::SelectModel => {
                if let Ok(model) = serde_json::from_value::<Model>(response.data)
                    && let Some(state) = self.active_snapshot_mut().session.as_mut()
                {
                    state.model = Some(model);
                }
                self.send(SessionCommand::ListReasoningLevels);
                self.send(SessionCommand::LoadState);
            }
            SessionOperation::SelectReasoning => {
                self.send(SessionCommand::LoadState);
            }
            SessionOperation::SelectMode => {
                self.send(SessionCommand::ListModes);
                self.send(SessionCommand::LoadState);
            }
            SessionOperation::Prompt(_) => {
                self.active_snapshot_mut().status = "Accepted".into();
                self.send(SessionCommand::LoadState);
            }
            SessionOperation::Abort => self.active_snapshot_mut().status = "Stopping".into(),
            SessionOperation::Compact => self.send(SessionCommand::LoadState),
            SessionOperation::Rename => {
                self.active_snapshot_mut().status = "Session named".into();
                self.send(SessionCommand::LoadState);
                self.refresh_sessions();
            }
            SessionOperation::ExportHtml => {
                self.active_snapshot_mut().status = response
                    .data
                    .get("path")
                    .and_then(Value::as_str)
                    .map_or_else(
                        || "Session exported".into(),
                        |path| format!("Exported to {path}"),
                    );
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
