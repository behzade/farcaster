use gpui::{Context, Entity, IntoElement as _, Render, Subscription, WeakEntity};

use super::PiApp;
use crate::app::workgraph::adapter::WorkGraphBoardView;
use crate::transcript;

pub(crate) struct SessionRailView {
    app: WeakEntity<PiApp>,
}

pub(crate) struct ArchivedSessionRailView {
    app: WeakEntity<PiApp>,
}

pub(crate) struct TranscriptView {
    app: WeakEntity<PiApp>,
    markdown_cache: crate::transcript_markdown::TranscriptMarkdownCache,
}

pub(crate) struct ComposerView {
    app: WeakEntity<PiApp>,
}

pub(crate) struct RunPanelView {
    app: WeakEntity<PiApp>,
}

pub(crate) struct WorkGraphDetailView {
    board: Entity<WorkGraphBoardView>,
    _board_subscription: Subscription,
}

impl SessionRailView {
    pub(crate) fn new(app: WeakEntity<PiApp>) -> Self {
        Self { app }
    }
}

impl ArchivedSessionRailView {
    pub(crate) fn new(app: WeakEntity<PiApp>) -> Self {
        Self { app }
    }
}

impl TranscriptView {
    pub(crate) fn new(app: WeakEntity<PiApp>) -> Self {
        Self {
            app,
            markdown_cache: crate::transcript_markdown::TranscriptMarkdownCache::default(),
        }
    }
}

impl ComposerView {
    pub(crate) fn new(app: WeakEntity<PiApp>) -> Self {
        Self { app }
    }
}

impl RunPanelView {
    pub(crate) fn new(app: WeakEntity<PiApp>) -> Self {
        Self { app }
    }
}

impl WorkGraphDetailView {
    pub(crate) fn new(board: Entity<WorkGraphBoardView>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&board, |_, _, cx| cx.notify());
        Self {
            board,
            _board_subscription: subscription,
        }
    }
}

impl Render for SessionRailView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let _timing = crate::performance::Timing::new("render.session_sidebar");
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        app.read(cx)
            .render_sessions(self.app.clone())
            .into_any_element()
    }
}

impl Render for ArchivedSessionRailView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let _timing = crate::performance::Timing::new("render.archived_session_sidebar");
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        app.read(cx)
            .render_archived_sessions(self.app.clone())
            .into_any_element()
    }
}

impl Render for TranscriptView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        let _timing = crate::performance::Timing::new("render.transcript");
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        let app = app.read(cx);
        let viewport = window.viewport_size();
        let layout_mode = crate::layout::layout_mode(viewport.width);
        let mut transcript_width = viewport.width;
        if crate::layout::shows_left_inline(layout_mode) {
            transcript_width -= crate::theme::THEME.layout.session_rail;
        }
        if crate::layout::shows_right_inline(layout_mode) {
            transcript_width -= crate::theme::THEME.layout.run_panel;
        }
        let diff_mode = if crate::layout::shows_split_diff(transcript_width) {
            crate::tool_changes::EmbeddedDiffMode::Split
        } else {
            crate::tool_changes::EmbeddedDiffMode::Unified
        };
        transcript::render(
            &app.transcript_list,
            transcript::TranscriptViewport {
                following: app.transcript_following,
                unseen: app.transcript_unseen,
                tail_reserve: transcript::tail_reserve(viewport.height),
                diff_mode,
            },
            app.transcript_rows.clone(),
            app.snapshot.clone(),
            app.transcript_disclosure_states.clone(),
            self.markdown_cache.clone(),
            self.app.clone(),
        )
        .into_any_element()
    }
}

impl Render for ComposerView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let _timing = crate::performance::Timing::new("render.composer");
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        app.read(cx)
            .render_composer(self.app.clone(), cx)
            .into_any_element()
    }
}

impl Render for WorkGraphDetailView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        self.board
            .update(cx, |board, cx| board.render_external_detail(cx))
    }
}

impl Render for RunPanelView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let _timing = crate::performance::Timing::new("render.run_sidebar");
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        app.read(cx)
            .render_run_panel(self.app.clone())
            .into_any_element()
    }
}
