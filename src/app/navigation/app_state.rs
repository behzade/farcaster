use crate::app::*;

pub(in crate::app) struct NavigationState {
    pub(in crate::app) picker: Option<navigation::PickerState>,
    pub(in crate::app) picker_return_focus: Option<FocusHandle>,
    pub(in crate::app) search: Entity<InputState>,
    pub(in crate::app) search_focus: FocusHandle,
    pub(in crate::app) chat: ui::navigation::ChatNavigation,
    pub(in crate::app) _search_subscription: Subscription,
}
