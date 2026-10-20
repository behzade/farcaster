use crate::{app::*, protocol::ExtensionUiRequest};
use gpui::App;

pub(in crate::app) struct ExtensionState {
    pub(in crate::app) active: ExtensionUiState,
    pub(in crate::app) question_sessions:
        HashMap<PathBuf, std::collections::BTreeMap<String, super::QuestionProgress>>,
    pub(in crate::app) parked: Option<ExtensionUiState>,
    pub(in crate::app) restored_dialog_id: Option<String>,
    pub(in crate::app) dismissed_restored_dialog_id: Option<String>,
    pub(in crate::app) notification_expiries: HashMap<(String, Instant), Task<()>>,
    pub(in crate::app) pending_dialog_setup: bool,
    pub(in crate::app) pending_title: Option<(u64, String)>,
    pub(in crate::app) pending_editor_text: Option<(u64, String)>,
    pub(in crate::app) dialog_input: Entity<TextareaState>,
    pub(in crate::app) question_input: Entity<gpui_component::input::InputState>,
    pub(in crate::app) question_input_key: Option<(String, String)>,
    pub(in crate::app) dialog_focus: FocusHandle,
    pub(in crate::app) dialog_return_focus: Option<FocusHandle>,
}

impl ExtensionState {
    pub(in crate::app) fn save_question_draft(&mut self, cx: &App) {
        if let Some(key) = self.question_input_key.take() {
            let value = self.question_input.read(cx).value().to_string();
            self.parked
                .as_mut()
                .unwrap_or(&mut self.active)
                .remember_question_draft(&key, value);
        }
    }

    pub(in crate::app) fn dialog_input_focus(&self, cx: &App) -> FocusHandle {
        match self.active.dialog {
            Some(ExtensionUiRequest::Questions(_))
                if self
                    .active
                    .current_question()
                    .is_some_and(|q| q.allow_free_text) =>
            {
                self.question_input.read(cx).focus_handle(cx)
            }
            Some(ExtensionUiRequest::Input { .. } | ExtensionUiRequest::Editor { .. }) => {
                self.dialog_input.read(cx).focus_handle(cx)
            }
            _ => self.dialog_focus.clone(),
        }
    }
}
