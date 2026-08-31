use gpui::{
    Context, Entity, IntoElement as _, ParentElement as _, Render, Styled as _, Subscription,
    WeakEntity, div,
};

use super::{FarcasterApp, SessionRailKind, transcript};
use crate::{
    app::ui::primitives::{ButtonTone, button},
    app::ui::theme::THEME,
    app::views::workgraph::WorkGraphBoardView,
};

pub(crate) struct SessionRailView {
    app: WeakEntity<FarcasterApp>,
}

pub(crate) struct InactiveSessionRailView {
    app: WeakEntity<FarcasterApp>,
    kind: SessionRailKind,
}

pub(crate) struct TranscriptView {
    app: WeakEntity<FarcasterApp>,
    markdown_cache: crate::app::views::transcript::markdown::TranscriptMarkdownCache,
}

pub(crate) struct ComposerView {
    app: WeakEntity<FarcasterApp>,
}

pub(crate) struct RunPanelView {
    app: WeakEntity<FarcasterApp>,
}

pub(crate) struct WorkGraphDetailView {
    app: WeakEntity<FarcasterApp>,
    board: Entity<WorkGraphBoardView>,
    _board_subscription: Subscription,
}

impl SessionRailView {
    pub(crate) fn new(app: WeakEntity<FarcasterApp>) -> Self {
        Self { app }
    }
}

impl InactiveSessionRailView {
    pub(crate) fn new(app: WeakEntity<FarcasterApp>, kind: SessionRailKind) -> Self {
        Self { app, kind }
    }
}

impl TranscriptView {
    pub(crate) fn new(app: WeakEntity<FarcasterApp>) -> Self {
        Self {
            app,
            markdown_cache:
                crate::app::views::transcript::markdown::TranscriptMarkdownCache::default(),
        }
    }
}

impl ComposerView {
    pub(crate) fn new(app: WeakEntity<FarcasterApp>) -> Self {
        Self { app }
    }
}

impl RunPanelView {
    pub(crate) fn new(app: WeakEntity<FarcasterApp>) -> Self {
        Self { app }
    }
}

impl WorkGraphDetailView {
    pub(crate) fn new(
        app: WeakEntity<FarcasterApp>,
        board: Entity<WorkGraphBoardView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&board, |_, _, cx| cx.notify());
        Self {
            app,
            board,
            _board_subscription: subscription,
        }
    }
}

impl Render for SessionRailView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let _timing =
            crate::app::infrastructure::performance::Timing::new("render.session_sidebar");
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        app.read(cx)
            .render_sessions(self.app.clone(), cx.has_active_drag())
            .into_any_element()
    }
}

impl Render for InactiveSessionRailView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let _timing =
            crate::app::infrastructure::performance::Timing::new("render.inactive_session_sidebar");
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        app.read(cx)
            .render_inactive_sessions(self.app.clone(), self.kind)
    }
}

impl Render for TranscriptView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        let _timing = crate::app::infrastructure::performance::Timing::new("render.transcript");
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        let app = app.read(cx);
        let viewport = window.viewport_size();
        transcript::render(
            &app.transcript_list,
            transcript::TranscriptViewport {
                following: app.transcript_following,
                unseen: app.transcript_unseen,
                tail_reserve: transcript::tail_reserve(viewport.height),
            },
            app.transcript_rows.clone(),
            app.snapshot.conversation.clone(),
            app.transcript_disclosure_states.clone(),
            self.markdown_cache.clone(),
            self.app.clone(),
        )
        .into_any_element()
    }
}

impl Render for ComposerView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let _timing = crate::app::infrastructure::performance::Timing::new("render.composer");
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
        let app = self.app.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .px(THEME.space.sm)
                    .py(THEME.space.xs)
                    .border_b(THEME.border)
                    .border_color(THEME.colors.border)
                    .child(button(
                        "close-workgraph-inspector",
                        "Back to session details",
                        ButtonTone::Quiet,
                        true,
                        move |_, cx| {
                            let _ = app.update(cx, |app, cx| {
                                app.close_workgraph_inspector(cx);
                            });
                        },
                    )),
            )
            .child(
                div().flex_1().min_h_0().child(
                    self.board
                        .update(cx, |board, cx| board.render_external_detail(cx)),
                ),
            )
    }
}

impl Render for RunPanelView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let _timing = crate::app::infrastructure::performance::Timing::new("render.run_sidebar");
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        app.read(cx)
            .render_run_panel(self.app.clone())
            .into_any_element()
    }
}
