#[derive(Default)]
pub(crate) struct OverlayViewState {
    pub(crate) pending_setup: bool,
    pub(crate) sessions: bool,
    pub(crate) run: bool,
    pub(crate) worker_notices: bool,
    pub(crate) keybindings: bool,
    pub(crate) settings: bool,
    pub(crate) project_trust: bool,
}
