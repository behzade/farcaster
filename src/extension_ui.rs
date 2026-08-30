use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

use crate::protocol::{ExtensionUiRequest, ExtensionUiResponse, NotifyTone, WidgetPlacement};

const MAX_NOTIFICATIONS: usize = 8;
const MAX_STATUSES: usize = 12;
const MAX_WIDGETS: usize = 8;
const MAX_WIDGET_LINES: usize = 24;
const MAX_WIDGET_LINE_CHARS: usize = 512;
const NOTIFICATION_LIFETIME: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Notification {
    pub id: String,
    pub message: String,
    pub tone: NotifyTone,
    pub expires_at: Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderAuthPrompt {
    pub url: String,
    pub message: String,
    status_key: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExtensionUiState {
    pub dialog: Option<ExtensionUiRequest>,
    queued_dialogs: VecDeque<ExtensionUiRequest>,
    pub provider_auth: Option<ProviderAuthPrompt>,
    pub notifications: VecDeque<Notification>,
    pub statuses: BTreeMap<String, String>,
    pub above_widgets: BTreeMap<String, Vec<String>>,
    pub below_widgets: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExtensionEffect {
    None,
    DialogOpened,
    SetTitle(String),
    SetEditorText(String),
    OpenUrl(String),
    PersistError(String),
    Diagnostic(String),
}

impl ExtensionUiState {
    pub(crate) fn apply(&mut self, request: ExtensionUiRequest) -> ExtensionEffect {
        match request {
            request @ (ExtensionUiRequest::Select { .. }
            | ExtensionUiRequest::Confirm { .. }
            | ExtensionUiRequest::Input { .. }
            | ExtensionUiRequest::Secret { .. }
            | ExtensionUiRequest::Editor { .. }) => {
                if self.dialog.is_none() {
                    self.dialog = Some(request);
                    ExtensionEffect::DialogOpened
                } else {
                    self.queued_dialogs.push_back(request);
                    ExtensionEffect::None
                }
            }
            ExtensionUiRequest::Notify { id, message, tone } => {
                if is_rpc_capability_notice(&message) {
                    return ExtensionEffect::None;
                }
                self.push_notification(id, message.clone(), tone);
                if tone == NotifyTone::Error {
                    ExtensionEffect::PersistError(message)
                } else {
                    ExtensionEffect::None
                }
            }
            ExtensionUiRequest::AuthUrl { url, message, .. } => {
                self.provider_auth = Some(ProviderAuthPrompt {
                    url: url.clone(),
                    message,
                    status_key: None,
                });
                ExtensionEffect::OpenUrl(url)
            }
            ExtensionUiRequest::SetStatus { key, text, .. } => {
                if let Some(text) = text {
                    if let Some(auth) = self.provider_auth.as_mut()
                        && auth.status_key.is_none()
                        && auth.message == text
                    {
                        auth.status_key = Some(key.clone());
                    }
                    self.statuses.insert(key, text);
                    trim_map(&mut self.statuses, MAX_STATUSES);
                } else {
                    self.statuses.remove(&key);
                    if self
                        .provider_auth
                        .as_ref()
                        .and_then(|auth| auth.status_key.as_ref())
                        == Some(&key)
                    {
                        self.provider_auth = None;
                    }
                }
                ExtensionEffect::None
            }
            ExtensionUiRequest::SetWidget {
                key,
                lines,
                placement,
                ..
            } => {
                let (target, other) = match placement {
                    WidgetPlacement::AboveEditor => {
                        (&mut self.above_widgets, &mut self.below_widgets)
                    }
                    WidgetPlacement::BelowEditor => {
                        (&mut self.below_widgets, &mut self.above_widgets)
                    }
                };
                other.remove(&key);
                if let Some(lines) = lines {
                    target.insert(key, bounded_widget_lines(lines));
                    trim_map(target, MAX_WIDGETS);
                } else {
                    target.remove(&key);
                }
                ExtensionEffect::None
            }
            ExtensionUiRequest::SetTitle { title, .. } => ExtensionEffect::SetTitle(title),
            ExtensionUiRequest::SetEditorText { text, .. } => ExtensionEffect::SetEditorText(text),
            ExtensionUiRequest::Unknown { method, .. } => {
                ExtensionEffect::Diagnostic(format!("Unknown extension UI method: {method}"))
            }
        }
    }

    fn push_notification(&mut self, id: String, message: String, tone: NotifyTone) {
        self.notifications.push_back(Notification {
            id,
            message,
            tone,
            expires_at: Instant::now() + NOTIFICATION_LIFETIME,
        });
        while self.notifications.len() > MAX_NOTIFICATIONS {
            self.notifications.pop_front();
        }
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn remove_notification(&mut self, id: &str, expires_at: Instant) -> bool {
        let before = self.notifications.len();
        self.notifications
            .retain(|notification| notification.id != id || notification.expires_at != expires_at);
        self.notifications.len() != before
    }

    pub(crate) fn respond_value(&mut self, id: &str, value: String) -> Option<ExtensionUiResponse> {
        self.take_dialog(id).map(|_| ExtensionUiResponse::Value {
            id: id.to_owned(),
            value,
        })
    }

    pub(crate) fn respond_confirm(
        &mut self,
        id: &str,
        confirmed: bool,
    ) -> Option<ExtensionUiResponse> {
        self.take_dialog(id)
            .map(|_| ExtensionUiResponse::Confirmed {
                id: id.to_owned(),
                confirmed,
            })
    }

    pub(crate) fn cancel(&mut self, id: &str) -> Option<ExtensionUiResponse> {
        self.take_dialog(id)
            .map(|_| ExtensionUiResponse::Cancelled {
                id: id.to_owned(),
                cancelled: true,
            })
    }

    fn take_dialog(&mut self, id: &str) -> Option<ExtensionUiRequest> {
        if self.dialog.as_ref().and_then(ExtensionUiRequest::dialog_id) != Some(id) {
            return None;
        }
        let taken = self.dialog.take();
        self.dialog = self.queued_dialogs.pop_front();
        taken
    }
}

fn is_rpc_capability_notice(message: &str) -> bool {
    message.ends_with(" not supported in RPC mode")
}

fn bounded_widget_lines(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .take(MAX_WIDGET_LINES)
        .map(|line| line.chars().take(MAX_WIDGET_LINE_CHARS).collect())
        .collect()
}

fn trim_map(map: &mut BTreeMap<String, impl Sized>, limit: usize) {
    while map.len() > limit {
        if let Some(first) = map.keys().next().cloned() {
            map.remove(&first);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(id: &str) -> ExtensionUiRequest {
        ExtensionUiRequest::Input {
            id: id.into(),
            title: "Value".into(),
            placeholder: None,
            timeout: None,
        }
    }

    #[test]
    fn rpc_capability_notices_are_not_user_facing_errors() {
        let mut state = ExtensionUiState::default();
        let effect = state.apply(ExtensionUiRequest::Notify {
            id: "notice".into(),
            message: "Theme switching not supported in RPC mode".into(),
            tone: NotifyTone::Error,
        });

        assert_eq!(effect, ExtensionEffect::None);
        assert!(state.notifications.is_empty());
    }

    #[test]
    fn notification_expiry_removes_only_the_matching_instance() {
        let mut state = ExtensionUiState::default();
        state.apply(ExtensionUiRequest::Notify {
            id: "notice".into(),
            message: "first".into(),
            tone: NotifyTone::Info,
        });
        let first_expiry = state.notifications[0].expires_at;
        state.apply(ExtensionUiRequest::Notify {
            id: "notice".into(),
            message: "replacement".into(),
            tone: NotifyTone::Info,
        });

        assert!(state.remove_notification("notice", first_expiry));
        assert_eq!(state.notifications.len(), 1);
        assert_eq!(state.notifications[0].message, "replacement");
        assert!(!state.remove_notification("notice", first_expiry));
    }

    #[test]
    fn keyed_status_and_widget_set_clear_and_bound_content() {
        let mut state = ExtensionUiState::default();
        state.apply(ExtensionUiRequest::SetStatus {
            id: "1".into(),
            key: "x".into(),
            text: Some("busy".into()),
        });
        state.apply(ExtensionUiRequest::SetWidget {
            id: "2".into(),
            key: "x".into(),
            lines: Some(vec![
                "x".repeat(MAX_WIDGET_LINE_CHARS + 10);
                MAX_WIDGET_LINES + 10
            ]),
            placement: WidgetPlacement::AboveEditor,
        });
        assert_eq!(state.statuses.get("x").map(String::as_str), Some("busy"));
        let lines = &state.above_widgets["x"];
        assert_eq!(lines.len(), MAX_WIDGET_LINES);
        assert_eq!(lines[0].chars().count(), MAX_WIDGET_LINE_CHARS);
        state.apply(ExtensionUiRequest::SetStatus {
            id: "3".into(),
            key: "x".into(),
            text: None,
        });
        state.apply(ExtensionUiRequest::SetWidget {
            id: "4".into(),
            key: "x".into(),
            lines: None,
            placement: WidgetPlacement::AboveEditor,
        });
        assert!(state.statuses.is_empty());
        assert!(state.above_widgets.is_empty());
    }

    #[test]
    fn dialog_requests_are_fifo_and_late_responses_are_ignored() {
        let mut state = ExtensionUiState::default();
        assert_eq!(state.apply(input("first")), ExtensionEffect::DialogOpened);
        assert_eq!(state.apply(input("second")), ExtensionEffect::None);
        assert!(state.respond_value("expired", "x".into()).is_none());
        assert!(state.respond_value("first", "x".into()).is_some());
        assert_eq!(
            state
                .dialog
                .as_ref()
                .and_then(ExtensionUiRequest::dialog_id),
            Some("second")
        );
        assert!(state.respond_value("first", "late".into()).is_none());
        assert!(state.cancel("second").is_some());
        assert!(state.dialog.is_none());
    }

    #[test]
    fn provider_auth_stays_visible_until_its_status_is_cleared() {
        let mut state = ExtensionUiState::default();
        let url = "https://x.ai/device";
        let message = "Enter code ABCD in your browser.";

        assert_eq!(
            state.apply(ExtensionUiRequest::AuthUrl {
                id: "auth".into(),
                url: url.into(),
                message: message.into(),
            }),
            ExtensionEffect::OpenUrl(url.into())
        );
        assert_eq!(
            state
                .provider_auth
                .as_ref()
                .map(|auth| auth.message.as_str()),
            Some(message)
        );
        state.apply(ExtensionUiRequest::SetStatus {
            id: "status".into(),
            key: "provider-login".into(),
            text: Some(message.into()),
        });
        state.apply(ExtensionUiRequest::SetStatus {
            id: "unrelated".into(),
            key: "other".into(),
            text: None,
        });
        assert!(state.provider_auth.is_some());

        state.apply(ExtensionUiRequest::SetStatus {
            id: "done".into(),
            key: "provider-login".into(),
            text: None,
        });
        assert!(state.provider_auth.is_none());
    }

    #[test]
    fn reset_clears_every_session_owned_surface() {
        let mut state = ExtensionUiState::default();
        state.apply(input("dialog"));
        state.apply(ExtensionUiRequest::SetStatus {
            id: "s".into(),
            key: "k".into(),
            text: Some("busy".into()),
        });
        state.apply(ExtensionUiRequest::Notify {
            id: "n".into(),
            message: "bad".into(),
            tone: NotifyTone::Error,
        });
        state.reset();
        assert_eq!(state, ExtensionUiState::default());
    }
}
