mod composer;
pub(in crate::app) mod dialogs;
mod regions;
mod root;
mod run_panel;
mod session_rail;
pub(crate) mod state;
pub(crate) mod transcript;
mod usage;
pub(super) mod workgraph;
mod workspace;

pub(super) use regions::{
    ComposerView, InactiveSessionRailView, RunPanelView, SessionRailView, TranscriptView,
    WorkGraphDetailView,
};
pub(in crate::app) use session_rail::{SessionRailKind, roots_waiting_for_descendants};

use super::FarcasterApp;

pub(crate) const OVERLAY_KEY_CONTEXT: &str = "FarcasterOverlay";
