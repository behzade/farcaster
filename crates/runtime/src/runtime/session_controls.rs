use super::*;
use crate::agents::Backend;

#[derive(Default)]
pub(super) struct PendingSessionControls {
    model: Option<(String, String)>,
    // Outer None means no pending change; Some(None) is an explicit reset.
    thinking: Option<Option<String>>,
    service_tier: Option<String>,
    model_requests: std::collections::HashSet<String>,
    sent_model: Option<(String, String)>,
    model_error: Option<String>,
    thinking_requests: std::collections::HashSet<String>,
    sent_thinking: Option<Option<String>>,
    thinking_error: Option<String>,
    tier_requests: std::collections::HashSet<String>,
    sent_tier: Option<String>,
    tier_error: Option<String>,
    pub(super) restore_preview: bool,
}

impl PendingSessionControls {
    fn clear_service_tier(&mut self) {
        self.service_tier = None;
        self.tier_error = None;
    }

    pub(super) fn launched_service_tier(&mut self, tier: &str) {
        if self.service_tier.as_deref() == Some(tier) {
            self.clear_service_tier();
        }
    }

    pub(super) fn model_pending(&self) -> bool {
        self.model.is_some() || !self.model_requests.is_empty()
    }

    pub(super) fn service_tier_pending(&self) -> bool {
        self.service_tier.is_some() || !self.tier_requests.is_empty()
    }

    pub(super) fn thinking_pending(&self) -> bool {
        self.thinking.is_some() || !self.thinking_requests.is_empty()
    }

    pub(super) fn selection_pending(&self) -> bool {
        self.model_pending() || self.thinking_pending() || self.service_tier_pending()
    }

    pub(super) fn selection_error(&self) -> Option<&str> {
        self.model_error
            .as_deref()
            .or(self.thinking_error.as_deref())
            .or(self.tier_error.as_deref())
    }

    pub(super) fn model_sent(&mut self, id: String, model: (String, String)) {
        self.model_requests.insert(id);
        self.sent_model = Some(model);
        self.model_error = None;
    }

    pub(super) fn tier_sent(&mut self, id: String, tier: String) {
        self.tier_requests.insert(id);
        self.sent_tier = Some(tier);
        self.tier_error = None;
    }

    pub(super) fn thinking_sent(&mut self, id: String, level: Option<String>) {
        self.thinking_requests.insert(id);
        self.sent_thinking = Some(level);
        self.thinking_error = None;
    }

    pub(super) fn reset_transport(&mut self) {
        if !self.model_requests.is_empty() {
            if self.model.is_none() {
                self.model = self.sent_model.take();
            }
            self.model_requests.clear();
        }
        if !self.thinking_requests.is_empty() {
            if self.thinking.is_none() {
                self.thinking = self.sent_thinking.take();
            }
            self.thinking_requests.clear();
        }
        if !self.tier_requests.is_empty() {
            if self.service_tier.is_none() {
                self.service_tier = self.sent_tier.take();
            }
            self.tier_requests.clear();
        }
    }

    pub(super) fn model_response(&mut self, response: &crate::agents::SessionResponse) {
        if let Some(id) = &response.id {
            self.model_requests.remove(id);
        }
        self.model_error = response
            .result
            .as_ref()
            .err()
            .map(|error| error.message.clone());
    }

    pub(super) fn tier_response(&mut self, response: &crate::agents::SessionResponse) {
        if let Some(id) = &response.id {
            self.tier_requests.remove(id);
        }
        self.tier_error = response
            .result
            .as_ref()
            .err()
            .map(|error| error.message.clone());
    }

    pub(super) fn thinking_response(
        &mut self,
        response: &crate::agents::SessionResponse,
    ) -> Option<Option<String>> {
        let matched = response
            .id
            .as_ref()
            .is_some_and(|id| self.thinking_requests.remove(id));
        self.thinking_error = response
            .result
            .as_ref()
            .err()
            .map(|error| error.message.clone());
        if matched && self.thinking_requests.is_empty() && response.result.is_ok() {
            self.sent_thinking.clone()
        } else {
            None
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.model.is_none() && self.thinking.is_none() && self.service_tier.is_none()
    }

    fn set(&mut self, control: SessionControl) {
        match control {
            SessionControl::Model(provider, model_id) => {
                self.model = Some((provider, model_id));
                self.model_error = None;
            }
            SessionControl::Thinking(level) => {
                self.thinking = Some(level);
                self.thinking_error = None;
            }
            SessionControl::ServiceTier(tier) => {
                self.service_tier = Some(tier);
                self.tier_error = None;
            }
        }
    }

    fn replace_if_pending(&mut self, control: SessionControl) -> Option<SessionControl> {
        if self.is_empty() {
            Some(control)
        } else {
            self.set(control);
            None
        }
    }

    fn take(&mut self) -> Vec<SessionControl> {
        let mut controls = Vec::with_capacity(2);
        if let Some((provider, model_id)) = self.model.take() {
            controls.push(SessionControl::Model(provider, model_id));
        }
        if let Some(level) = self.thinking.take() {
            controls.push(SessionControl::Thinking(level));
        }
        if let Some(tier) = self.service_tier.take() {
            controls.push(SessionControl::ServiceTier(tier));
        }
        controls
    }
}

enum SessionControl {
    Model(String, String),
    Thinking(Option<String>),
    ServiceTier(String),
}

impl SessionControl {
    fn supported_by(&self, harness: impl Into<Option<Backend>>) -> bool {
        let Some(harness) = harness.into() else {
            return false;
        };
        match self {
            Self::Thinking(None) => crate::agents::supports_reasoning_reset(harness),
            Self::Thinking(Some(_)) => crate::agents::supports_reasoning_effort(harness),
            _ => true,
        }
    }

    fn command_name(&self) -> &'static str {
        match self {
            Self::Model(..) => "set_model",
            Self::Thinking(_) => "set_thinking_level",
            Self::ServiceTier(_) => "set_service_tier",
        }
    }

    fn into_request(self) -> SessionCommand {
        match self {
            Self::Model(provider, model_id) => SessionCommand::SelectModel { provider, model_id },
            Self::Thinking(Some(level)) => SessionCommand::SelectReasoning { level },
            Self::Thinking(None) => SessionCommand::ResetReasoning,
            Self::ServiceTier(tier) => SessionCommand::SelectServiceTier { tier },
        }
    }
}

impl RuntimeOwner {
    pub(super) fn set_model(&mut self, model: Model) {
        let available = crate::agents::available_access_modes(
            self.harness,
            Some(self.snapshot.catalog_model(&model)),
            self.selected_sandbox_adapter().as_deref(),
        );
        let current = self.process_command.access_mode;
        let requested = self.access_mode_changes.requested_mode(current);
        if self.process.is_some() {
            if !available.contains(&current) || !available.contains(&requested) {
                self.command_not_sent(
                    "set_model",
                    "Change the sandbox mode before selecting this model",
                );
                return;
            }
        } else {
            let Some(mode) = self
                .access_mode_changes
                .resolve_available(current, &available)
            else {
                self.command_not_sent("set_model", "No access mode is available for this model");
                return;
            };
            self.process_command.access_mode = mode;
        }
        let replacement_effort = super::session_identity::replacement_effort(
            &model,
            self.snapshot.session_identity().effort,
        );
        let control = SessionControl::Model(model.provider.clone(), model.id.clone());
        if !self.snapshot.history_preview
            && self.process.is_none()
            && self.snapshot.selected_session.is_none()
        {
            self.snapshot.prefill_model = Some(model);
            self.pending_session_controls.set(control);
            if let Some(effort) = replacement_effort {
                self.snapshot.prefill_thinking_level = Some(effort.clone());
                self.pending_session_controls
                    .set(SessionControl::Thinking(Some(effort)));
            }
            self.publish();
            return;
        }
        self.remember_requested_model(&model, replacement_effort.as_deref());
        self.send_session_control(control);
        if let Some(effort) = replacement_effort {
            self.send_session_control(SessionControl::Thinking(Some(effort)));
        }
    }

    fn remember_requested_model(&mut self, model: &Model, effort: Option<&str>) {
        let update = |snapshot: &mut RuntimeSnapshot| {
            snapshot.prefill_model = Some(model.clone());
            snapshot.pending_initial_model = true;
            if let Some(effort) = effort {
                snapshot.prefill_thinking_level = Some(effort.to_owned());
            }
        };
        update(&mut self.snapshot);
        if self.snapshot.history_preview
            && self.active_session.as_ref() == self.snapshot.selected_session.as_ref()
            && let Some(loading) = self.parked_snapshot.as_mut()
        {
            update(loading);
        }
    }

    pub(super) fn set_thinking(&mut self, level: String) {
        self.send_session_control(SessionControl::Thinking(Some(level)));
    }

    pub(super) fn reset_thinking(&mut self) {
        self.send_session_control(SessionControl::Thinking(None));
    }

    pub(super) fn set_service_tier(&mut self, tier: String) {
        if !self.snapshot.available_service_tiers().contains(&tier) {
            self.command_not_sent(
                "set_service_tier",
                "Service tier is not available for this model",
            );
            return;
        }
        if matches!(self.harness, Some(Backend::Codex | Backend::Claude))
            && (self.process.is_some() || self.snapshot.selected_session.is_some())
        {
            if self.process.is_some() && !self.access_mode_change_ready() {
                self.command_not_sent(
                    "set_service_tier",
                    "Wait for the current response to finish before changing service tier",
                );
                return;
            }
            let session = if self.snapshot.history_preview {
                self.snapshot.selected_session.clone()
            } else {
                self.active_session
                    .clone()
                    .or_else(|| self.snapshot.selected_session.clone())
            };
            let Some(session) = session else {
                self.command_not_sent("set_service_tier", "No session is selected");
                return;
            };
            self.queue_launch_only_service_tier(tier);
            let preserve_transcript = self.process.is_some() && !self.snapshot.history_preview;
            self.start_process_from(Some(session), None, preserve_transcript);
            return;
        }
        self.send_session_control(SessionControl::ServiceTier(tier));
    }

    fn queue_launch_only_service_tier(&mut self, tier: String) {
        if self.process.is_some() && !self.snapshot.history_preview {
            let model = self.active_snapshot().session_identity().model.cloned();
            let effort = self
                .active_snapshot()
                .session_identity()
                .effort
                .map(str::to_owned);
            if !self.pending_session_controls.model_pending()
                && let Some(model) = model
            {
                self.pending_session_controls
                    .set(SessionControl::Model(model.provider, model.id));
            }
            if !self.pending_session_controls.thinking_pending()
                && let Some(effort) = effort
            {
                self.pending_session_controls
                    .set(SessionControl::Thinking(Some(effort)));
            }
        }
        self.snapshot.prefill_service_tier = Some(tier.clone());
        self.snapshot.pending_initial_service_tier = true;
        self.pending_session_controls
            .set(SessionControl::ServiceTier(tier));
    }

    fn send_session_control(&mut self, control: SessionControl) {
        if !control.supported_by(self.harness) {
            return;
        }
        if !self.snapshot.history_preview && self.process.is_some() {
            if let Some(control) = self.pending_session_controls.replace_if_pending(control) {
                self.send(control.into_request());
            }
            return;
        }
        if !self.snapshot.history_preview
            && self.process.is_none()
            && self.snapshot.selected_session.is_none()
        {
            match &control {
                SessionControl::Model(provider, model_id) => {
                    self.snapshot.prefill_model = self
                        .snapshot
                        .models
                        .iter()
                        .find(|model| model.provider == *provider && model.id == *model_id)
                        .cloned();
                }
                SessionControl::Thinking(level) => {
                    self.snapshot.prefill_thinking_level = level.clone();
                }
                SessionControl::ServiceTier(tier) => {
                    self.snapshot.prefill_service_tier = Some(tier.clone());
                    self.snapshot.pending_initial_service_tier = true;
                }
            }
            self.pending_session_controls.set(control);
            self.publish();
            return;
        }
        let Some(session) = self.snapshot.selected_session.clone() else {
            self.command_not_sent(control.command_name(), "No session is selected");
            return;
        };
        let reconnecting = !self.pending_session_controls.is_empty() && self.process.is_some();
        self.pending_session_controls.set(control);
        if !reconnecting {
            self.start_process(Some(session));
        }
    }

    pub(super) fn maybe_send_pending_session_controls(&mut self) {
        if !self.startup_state_loaded || !self.startup_history_loaded {
            return;
        }
        let resuming_preview = self.snapshot.history_preview
            && self.parked_snapshot.is_some()
            && self.pending_session_controls.restore_preview;
        if self.pending_session_controls.is_empty() && !resuming_preview {
            self.pending_session_controls.restore_preview = false;
            return;
        }
        let controls = self.pending_session_controls.take();
        if resuming_preview && let Some(snapshot) = self.parked_snapshot.take() {
            self.snapshot = snapshot;
        }
        self.pending_session_controls.restore_preview = false;
        for control in controls {
            if self.process.is_none() {
                break;
            }
            if control.supported_by(self.harness) {
                self.send(control.into_request());
            }
        }
    }

    pub(super) fn fail_session_control_resume(
        &mut self,
        status: &str,
        label: &str,
        details: String,
    ) {
        self.pending_session_controls = PendingSessionControls::default();
        if let Some(mut process) = self.process.take() {
            let _ = process.close();
        }
        self.active_session = None;
        self.parked_snapshot = None;
        self.snapshot.connected = false;
        self.snapshot.status = status.into();
        let conversation = conversation_mut(&mut self.snapshot);
        conversation.diagnostics.push(details.clone());
        conversation.push_local_error_with_details(label, failure_summary(&details), details);
        self.publish();
    }

    fn command_not_sent(&mut self, command_name: &str, reason: &str) {
        let snapshot = &mut self.snapshot;
        conversation_mut(snapshot)
            .push_local_error("Command not sent", format!("{command_name}: {reason}"));
        snapshot.status = "Command not sent".into();
        self.publish();
    }
}

#[cfg(test)]
#[path = "session_controls_tests.rs"]
mod tests;
