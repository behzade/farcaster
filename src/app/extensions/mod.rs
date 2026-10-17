mod app_state;
mod state;

pub(in crate::app) use app_state::ExtensionState;

use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

use crate::protocol::{ExtensionUiRequest, ExtensionUiResponse, NotifyTone, WidgetPlacement};

const MAX_NOTIFICATIONS: usize = 8;
const MAX_NOTIFICATION_HISTORY: usize = 200;
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
pub(crate) struct NotificationRecord {
    pub id: String,
    pub message: String,
    pub tone: NotifyTone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct QuestionProgress {
    request: crate::agents::QuestionRequest,
    answers: BTreeMap<String, Vec<String>>,
    drafts: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExtensionUiState {
    pub dialog: Option<ExtensionUiRequest>,
    queued_dialogs: VecDeque<ExtensionUiRequest>,
    pub(in crate::app) question_progress: BTreeMap<String, QuestionProgress>,
    pub notifications: VecDeque<Notification>,
    pub notification_history: VecDeque<NotificationRecord>,
    unseen_notifications: usize,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DialogDismissal {
    NotFound,
    Queued,
    ActiveWithNext,
    ActiveFinal,
}

impl ExtensionUiState {
    pub(crate) fn apply(&mut self, request: ExtensionUiRequest) -> ExtensionEffect {
        match request {
            request @ (ExtensionUiRequest::Questions(_)
            | ExtensionUiRequest::WorkerModel { .. }
            | ExtensionUiRequest::Select { .. }
            | ExtensionUiRequest::Confirm { .. }
            | ExtensionUiRequest::Input { .. }
            | ExtensionUiRequest::Editor { .. }) => {
                if self.dialog.as_ref().and_then(ExtensionUiRequest::dialog_id)
                    == request.dialog_id()
                    || self
                        .queued_dialogs
                        .iter()
                        .any(|q| q.dialog_id() == request.dialog_id())
                {
                    return ExtensionEffect::None;
                }
                if let ExtensionUiRequest::Questions(batch) = &request
                    && self
                        .question_progress
                        .get(&batch.id)
                        .is_none_or(|saved| saved.request != *batch)
                {
                    self.question_progress.insert(
                        batch.id.clone(),
                        QuestionProgress {
                            request: batch.clone(),
                            answers: BTreeMap::new(),
                            drafts: BTreeMap::new(),
                        },
                    );
                }
                if request.is_blocking() && self.dialog.as_ref().is_some_and(|d| !d.is_blocking()) {
                    if let Some(previous) = self.dialog.take() {
                        self.queued_dialogs.push_front(previous);
                    }
                }
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
            id: id.clone(),
            message: message.clone(),
            tone,
            expires_at: Instant::now() + NOTIFICATION_LIFETIME,
        });
        while self.notifications.len() > MAX_NOTIFICATIONS {
            self.notifications.pop_front();
        }
        self.notification_history
            .push_back(NotificationRecord { id, message, tone });
        while self.notification_history.len() > MAX_NOTIFICATION_HISTORY {
            self.notification_history.pop_front();
        }
        self.unseen_notifications = self.unseen_notifications.saturating_add(1);
    }

    pub(crate) fn unseen_notifications(&self) -> usize {
        self.unseen_notifications
    }

    pub(crate) fn mark_notifications_seen(&mut self) {
        self.unseen_notifications = 0;
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

    pub(crate) fn current_question(&self) -> Option<&crate::agents::Question> {
        let ExtensionUiRequest::Questions(request) = self.dialog.as_ref()? else {
            return None;
        };
        let answers = self
            .question_progress
            .get(&request.id)
            .map(|progress| &progress.answers);
        request
            .questions
            .iter()
            .find(|question| answers.is_none_or(|a| !a.contains_key(&question.id)))
    }

    pub(crate) fn remember_question_draft(&mut self, key: &(String, String), value: String) {
        if let Some(progress) = self.question_progress.get_mut(&key.0)
            && progress.request.questions.iter().any(|q| q.id == key.1)
            && !progress.answers.contains_key(&key.1)
        {
            progress.drafts.insert(key.1.clone(), value);
        }
    }

    pub(crate) fn question_draft(&self, key: &(String, String)) -> String {
        self.question_progress
            .get(&key.0)
            .and_then(|progress| progress.drafts.get(&key.1))
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn answer_question(
        &mut self,
        id: &str,
        question_id: &str,
        value: String,
    ) -> Result<Option<ExtensionUiResponse>, ()> {
        if self.dialog.as_ref().and_then(ExtensionUiRequest::dialog_id) != Some(id) {
            return Err(());
        }
        let question = self.current_question().ok_or(())?;
        if question.id != question_id
            || value.trim().is_empty()
            || (!question.allow_free_text && !question.options.iter().any(|o| o.label == value))
        {
            return Err(());
        }
        let question_id = question.id.clone();
        let progress = self.question_progress.get_mut(id).ok_or(())?;
        progress.drafts.remove(&question_id);
        progress.answers.insert(question_id, vec![value]);
        if self.current_question().is_some() {
            return Ok(None);
        }
        let answers = self.question_progress.remove(id).ok_or(())?.answers;
        self.take_dialog(id);
        Ok(Some(ExtensionUiResponse::Questions(
            crate::agents::QuestionResponse {
                id: id.to_owned(),
                answers,
                cancelled: false,
            },
        )))
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
        self.take_dialog(id).map(|request| match request {
            ExtensionUiRequest::Questions(_) => {
                ExtensionUiResponse::Questions(crate::agents::QuestionResponse {
                    id: id.to_owned(),
                    answers: BTreeMap::new(),
                    cancelled: true,
                })
            }
            _ => ExtensionUiResponse::Cancelled {
                id: id.to_owned(),
                cancelled: true,
            },
        })
    }

    pub(crate) fn dismiss_dialog(&mut self, id: &str) -> DialogDismissal {
        if self.dialog.as_ref().and_then(ExtensionUiRequest::dialog_id) == Some(id) {
            self.take_dialog(id);
            return if self.dialog.is_some() {
                DialogDismissal::ActiveWithNext
            } else {
                DialogDismissal::ActiveFinal
            };
        }
        self.question_progress.remove(id);
        let queued = self.queued_dialogs.len();
        self.queued_dialogs
            .retain(|request| request.dialog_id() != Some(id));
        if self.queued_dialogs.len() != queued {
            DialogDismissal::Queued
        } else {
            DialogDismissal::NotFound
        }
    }

    fn take_dialog(&mut self, id: &str) -> Option<ExtensionUiRequest> {
        if self.dialog.as_ref().and_then(ExtensionUiRequest::dialog_id) != Some(id) {
            return None;
        }
        self.question_progress.remove(id);
        let taken = self.dialog.take();
        self.dialog = self
            .queued_dialogs
            .iter()
            .position(ExtensionUiRequest::is_blocking)
            .and_then(|index| self.queued_dialogs.remove(index))
            .or_else(|| self.queued_dialogs.pop_front());
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
