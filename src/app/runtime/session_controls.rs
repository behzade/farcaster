use super::*;

#[derive(Default)]
pub(super) struct PendingSessionControls {
    model: Option<(String, String)>,
    thinking: Option<String>,
    service_tier: Option<String>,
    model_requests: std::collections::HashSet<String>,
    sent_model: Option<(String, String)>,
    model_error: Option<String>,
}

impl PendingSessionControls {
    pub(super) fn model_pending(&self) -> bool {
        self.model.is_some() || !self.model_requests.is_empty()
    }

    pub(super) fn model_error(&self) -> Option<&str> {
        self.model_error.as_deref()
    }

    pub(super) fn model_sent(&mut self, id: String, model: (String, String)) {
        self.model_requests.insert(id);
        self.sent_model = Some(model);
        self.model_error = None;
    }

    pub(super) fn reset_transport(&mut self) {
        if !self.model_requests.is_empty() {
            if self.model.is_none() {
                self.model = self.sent_model.take();
            }
            self.model_requests.clear();
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

    pub(super) fn is_empty(&self) -> bool {
        self.model.is_none() && self.thinking.is_none() && self.service_tier.is_none()
    }

    fn set(&mut self, control: SessionControl) {
        match control {
            SessionControl::Model(provider, model_id) => {
                self.model = Some((provider, model_id));
                self.model_error = None;
            }
            SessionControl::Thinking(level) => self.thinking = Some(level),
            SessionControl::ServiceTier(tier) => self.service_tier = Some(tier),
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
    Thinking(String),
    ServiceTier(String),
}

impl SessionControl {
    fn supported_by(&self, harness: &str) -> bool {
        !matches!(self, Self::Thinking(_)) || crate::agents::supports_reasoning_effort(harness)
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
            Self::Thinking(level) => SessionCommand::SelectReasoning { level },
            Self::ServiceTier(tier) => SessionCommand::SelectServiceTier { tier },
        }
    }
}

impl RuntimeOwner {
    pub(super) fn set_model(&mut self, model: Model) {
        let available = crate::agents::available_access_modes(
            &self.harness,
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
                    .set(SessionControl::Thinking(effort));
            }
            self.publish();
            return;
        }
        self.send_session_control(control);
        if let Some(effort) = replacement_effort {
            self.send_session_control(SessionControl::Thinking(effort));
        }
    }

    pub(super) fn set_thinking(&mut self, level: String) {
        self.send_session_control(SessionControl::Thinking(level));
    }

    pub(super) fn set_service_tier(&mut self, tier: String) {
        if !self
            .snapshot
            .session
            .as_ref()
            .is_some_and(|state| state.service_tiers.contains(&tier))
        {
            self.command_not_sent(
                "set_service_tier",
                "Service tier is not available for this model",
            );
            return;
        }
        self.send_session_control(SessionControl::ServiceTier(tier));
    }

    fn send_session_control(&mut self, control: SessionControl) {
        if !control.supported_by(&self.harness) {
            return;
        }
        if !self.snapshot.history_preview && self.process.is_some() {
            self.send(control.into_request());
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
                    self.snapshot.prefill_thinking_level = Some(level.clone());
                }
                SessionControl::ServiceTier(_) => {}
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
        if !self.startup_state_loaded
            || !self.startup_history_loaded
            || self.pending_session_controls.is_empty()
        {
            return;
        }
        let controls = self.pending_session_controls.take();
        if self.snapshot.history_preview
            && let Some(snapshot) = self.parked_snapshot.take()
        {
            self.snapshot = snapshot;
        }
        for control in controls {
            if self.process.is_none() {
                break;
            }
            if control.supported_by(&self.harness) {
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
