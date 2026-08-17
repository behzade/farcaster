use gpui::{Context, IntoElement as _, Render, WeakEntity};

use super::PiApp;
use crate::transcript;

pub(crate) struct SessionRailView {
    app: WeakEntity<PiApp>,
}

pub(crate) struct TranscriptView {
    app: WeakEntity<PiApp>,
}

pub(crate) struct ComposerView {
    app: WeakEntity<PiApp>,
}

pub(crate) struct RunPanelView {
    app: WeakEntity<PiApp>,
}

impl SessionRailView {
    pub(crate) fn new(app: WeakEntity<PiApp>) -> Self {
        Self { app }
    }
}

impl TranscriptView {
    pub(crate) fn new(app: WeakEntity<PiApp>) -> Self {
        Self { app }
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

impl Render for SessionRailView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        app.read(cx)
            .render_sessions(self.app.clone())
            .into_any_element()
    }
}

impl Render for TranscriptView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        let app = app.read(cx);
        transcript::render(
            &app.transcript_list,
            transcript::TranscriptViewport {
                following: app.transcript_following,
                unseen: app.transcript_unseen,
                tail_reserve: transcript::tail_reserve(window.viewport_size().height),
            },
            app.transcript_rows.clone(),
            app.snapshot.clone(),
            app.transcript_disclosure_overrides.clone(),
            self.app.clone(),
        )
        .into_any_element()
    }
}

impl Render for ComposerView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        app.read(cx)
            .render_composer(self.app.clone(), cx)
            .into_any_element()
    }
}

impl Render for RunPanelView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        app.read(cx)
            .render_run_panel(self.app.clone())
            .into_any_element()
    }
}
