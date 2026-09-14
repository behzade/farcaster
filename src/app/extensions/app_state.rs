use crate::app::*;

pub(in crate::app) struct ExtensionState {
    pub(in crate::app) active: ExtensionUiState,
    pub(in crate::app) parked: Option<ExtensionUiState>,
    pub(in crate::app) restored_dialog_id: Option<String>,
    pub(in crate::app) dismissed_restored_dialog_id: Option<String>,
    pub(in crate::app) notification_expiries: HashMap<(String, Instant), Task<()>>,
    pub(in crate::app) pending_dialog_setup: bool,
    pub(in crate::app) pending_title: Option<(u64, String)>,
    pub(in crate::app) pending_editor_text: Option<(u64, String)>,
    pub(in crate::app) dialog_input: Entity<TextareaState>,
    pub(in crate::app) dialog_focus: FocusHandle,
    pub(in crate::app) dialog_return_focus: Option<FocusHandle>,
}
