use crate::app::*;

pub(in crate::app) struct NavigationState {
    pub(in crate::app) picker: Option<navigation::PickerState>,
    pub(in crate::app) pending_model_access: Option<PendingModelAccess>,
    pub(in crate::app) picker_return_focus: Option<FocusHandle>,
    pub(in crate::app) search: Entity<InputState>,
    pub(in crate::app) search_focus: FocusHandle,
    pub(in crate::app) chat: ui::navigation::ChatNavigation,
    pub(in crate::app) _search_subscription: Subscription,
}

pub(in crate::app) struct PendingModelAccess {
    pub(in crate::app) focus: FocusHandle,
    pub(in crate::app) model: Model,
    pub(in crate::app) modes: Vec<runtime::HarnessAccessMode>,
    pub(in crate::app) effort: Option<String>,
    pub(in crate::app) apply_effort: bool,
    pub(in crate::app) return_focus: Option<FocusHandle>,
}
