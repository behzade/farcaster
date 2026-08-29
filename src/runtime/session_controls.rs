//! Session control commands that may need to resume a history-only document.

use super::*;

#[derive(Default)]
pub(super) struct PendingSessionControls {
    model: Option<(String, String)>,
    thinking: Option<String>,
}

impl PendingSessionControls {
    pub(super) fn is_empty(&self) -> bool {
        self.model.is_none() && self.thinking.is_none()
    }

    fn set(&mut self, control: SessionControl) {
        match control {
            SessionControl::Model(provider, model_id) => {
                self.model = Some((provider, model_id));
            }
            SessionControl::Thinking(level) => self.thinking = Some(level),
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
        controls
    }
}

enum SessionControl {
    Model(String, String),
    Thinking(String),
}

impl SessionControl {
    fn command_name(&self) -> &'static str {
        match self {
            Self::Model(..) => "set_model",
            Self::Thinking(_) => "set_thinking_level",
        }
    }

    fn into_request(self) -> BackendRequest {
        match self {
            Self::Model(provider, model_id) => BackendRequest::SelectModel { provider, model_id },
            Self::Thinking(level) => BackendRequest::SelectReasoning { level },
        }
    }
}

impl RuntimeOwner {
    pub(super) fn set_model(&mut self, provider: String, model_id: String) {
        self.send_session_control(SessionControl::Model(provider, model_id));
    }

    pub(super) fn set_thinking(&mut self, level: String) {
        self.send_session_control(SessionControl::Thinking(level));
    }

    fn send_session_control(&mut self, control: SessionControl) {
        if !self.snapshot.history_preview && self.process.is_some() {
            self.send(control.into_request());
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
            self.send(control.into_request());
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
mod tests {
    use super::*;

    #[test]
    fn pending_controls_coalesce_and_apply_model_before_effort() {
        let mut pending = PendingSessionControls::default();
        pending.set(SessionControl::Thinking("low".into()));
        pending.set(SessionControl::Model(
            "old-provider".into(),
            "old-model".into(),
        ));
        pending.set(SessionControl::Thinking("high".into()));
        pending.set(SessionControl::Model(
            "new-provider".into(),
            "new-model".into(),
        ));

        let requests = pending
            .take()
            .into_iter()
            .map(SessionControl::into_request)
            .collect::<Vec<_>>();

        assert_eq!(
            requests,
            vec![
                BackendRequest::SelectModel {
                    provider: "new-provider".into(),
                    model_id: "new-model".into(),
                },
                BackendRequest::SelectReasoning {
                    level: "high".into(),
                },
            ]
        );
    }
}
