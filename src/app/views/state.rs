use std::{collections::HashMap, sync::Arc};

use gpui::{Pixels, ScrollHandle};

use super::transcript::TranscriptRow;
use crate::app::ui::persistent_vec::PersistentVec;

pub(crate) struct ViewState {
    pub(crate) composer: ComposerViewState,
    pub(crate) overlays: OverlayViewState,
    pub(crate) run_panel: RunPanelViewState,
    pub(crate) session_rail: SessionRailViewState,
    pub(crate) transcript: TranscriptViewState,
}

impl ViewState {
    pub(crate) fn new(transcript_list: super::transcript::list::TranscriptListState) -> Self {
        Self {
            composer: ComposerViewState {
                suggestion_selection: 0,
                footer_scroll: ScrollHandle::new(),
            },
            overlays: OverlayViewState::default(),
            run_panel: RunPanelViewState {
                width: super::super::ui::theme::THEME.layout.run_panel,
                resize_start: None,
                scroll: ScrollHandle::new(),
                completed_agents_expanded: false,
                limited_agents_expanded: false,
            },
            session_rail: SessionRailViewState {
                width: super::super::ui::theme::THEME.layout.session_rail,
                resize_start: None,
                shortcuts_visible: false,
            },
            transcript: TranscriptViewState {
                list: transcript_list,
                rows: Arc::new(PersistentVec::default()),
                following: true,
                unseen: 0,
                disclosure_states: HashMap::new(),
                last_count: 0,
            },
        }
    }
}

pub(crate) struct ComposerViewState {
    pub(crate) suggestion_selection: usize,
    pub(crate) footer_scroll: ScrollHandle,
}

#[derive(Default)]
pub(crate) struct OverlayViewState {
    pub(crate) pending_setup: bool,
    pub(crate) sessions: bool,
    pub(crate) run: bool,
    pub(crate) keybindings: bool,
    pub(crate) settings: bool,
    pub(crate) project_trust: bool,
}

pub(crate) struct RunPanelViewState {
    pub(crate) width: Pixels,
    pub(crate) resize_start: Option<(Pixels, Pixels)>,
    pub(crate) scroll: ScrollHandle,
    pub(crate) completed_agents_expanded: bool,
    pub(crate) limited_agents_expanded: bool,
}

pub(crate) struct SessionRailViewState {
    pub(crate) width: Pixels,
    pub(crate) resize_start: Option<(Pixels, Pixels)>,
    pub(crate) shortcuts_visible: bool,
}

pub(crate) struct TranscriptViewState {
    pub(crate) list: super::transcript::list::TranscriptListState,
    pub(crate) rows: Arc<PersistentVec<TranscriptRow>>,
    pub(crate) following: bool,
    pub(crate) unseen: usize,
    pub(crate) disclosure_states: HashMap<usize, bool>,
    pub(crate) last_count: usize,
}
