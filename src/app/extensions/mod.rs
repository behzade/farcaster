mod state;

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExtensionUiState {
    pub dialog: Option<ExtensionUiRequest>,
    queued_dialogs: VecDeque<ExtensionUiRequest>,
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
    PersistError(String),
    Diagnostic(String),
}

impl ExtensionUiState {
    pub(crate) fn apply(&mut self, request: ExtensionUiRequest) -> ExtensionEffect {
        match request {
            request @ (ExtensionUiRequest::Select { .. }
            | ExtensionUiRequest::Confirm { .. }
            | ExtensionUiRequest::Input { .. }
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
            ExtensionUiRequest::SetStatus { key, text, .. } => {
                if let Some(text) = text {
                    self.statuses.insert(key, text);
                    trim_map(&mut self.statuses, MAX_STATUSES);
                } else {
                    self.statuses.remove(&key);
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

    pub(crate) fn push_notification(&mut self, id: String, message: String, tone: NotifyTone) {
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
#[path = "mod_tests.rs"]
mod tests;
