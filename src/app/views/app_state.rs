use crate::app::{ui::primitives::ResizeState, *};

pub(in crate::app) struct AppViews {
    pub(in crate::app) session_rail: Entity<SessionRailView>,
    pub(in crate::app) archived_session_rail: Entity<InactiveSessionRailView>,
    pub(in crate::app) transcript: Entity<TranscriptView>,
    pub(in crate::app) composer: Entity<ComposerView>,
    pub(in crate::app) run_panel: Entity<RunPanelView>,
    pub(in crate::app) workgraph: Entity<WorkGraphBoardView>,
    pub(in crate::app) workgraph_detail: Entity<WorkGraphDetailView>,
    pub(in crate::app) workgraph_sidebar: Entity<WorkGraphSidebarView>,
    pub(in crate::app) workgraph_inspector_issue: Option<u64>,
    pub(in crate::app) notification_panel: ResizeState,
    pub(in crate::app) archived_panel: ResizeState,
}

pub(in crate::app) struct AppOverlays {
    pub(in crate::app) view: views::overlay_state::OverlayViewState,
    pub(in crate::app) image_preview: Option<ImagePreview>,
    pub(in crate::app) image_preview_focus: FocusHandle,
    pub(in crate::app) image_preview_return_focus: Option<FocusHandle>,
    pub(in crate::app) sheet_focus: FocusHandle,
    pub(in crate::app) sheet_return_focus: Option<FocusHandle>,
    pub(in crate::app) post_render_focus: Option<PostRenderFocus>,
}
