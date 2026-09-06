use gpui::{Context, IntoElement as _, Render, ScrollHandle, WeakEntity};

use super::super::FarcasterApp;

pub(crate) struct ComposerView {
    app: WeakEntity<FarcasterApp>,
    suggestion_selection: usize,
    footer_scroll: ScrollHandle,
    status_scroll: ScrollHandle,
}

impl ComposerView {
    pub(crate) fn new(app: WeakEntity<FarcasterApp>) -> Self {
        Self {
            app,
            suggestion_selection: 0,
            footer_scroll: ScrollHandle::new(),
            status_scroll: ScrollHandle::new(),
        }
    }

    pub(crate) fn suggestion_selection(&self) -> usize {
        self.suggestion_selection
    }

    pub(crate) fn reset_suggestion_selection(&mut self) {
        self.suggestion_selection = 0;
    }

    pub(crate) fn select_previous_suggestion(
        &mut self,
        count: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        if count == 0 {
            return false;
        }
        self.suggestion_selection = self
            .suggestion_selection
            .checked_sub(1)
            .unwrap_or(count - 1);
        cx.notify();
        true
    }

    pub(crate) fn select_next_suggestion(&mut self, count: usize, cx: &mut Context<Self>) -> bool {
        if count == 0 {
            return false;
        }
        self.suggestion_selection = (self.suggestion_selection + 1) % count;
        cx.notify();
        true
    }
}

impl Render for ComposerView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        let _timing = crate::app::infrastructure::performance::Timing::new("render.composer");
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        let app = app.read(cx);
        let mode = if let Some(hint) = app.chat_navigation.activation.hint() {
            Some(hint)
        } else if app.chat_navigation.focus.is_focused(window) {
            Some(if app.chat_navigation.leader_pending {
                "SPACE · e editor · t terminal · j/k sessions · Esc cancel"
            } else {
                "NORMAL · CHAT"
            })
        } else if app.composer_focus.is_focused(window) {
            Some("INSERT · COMPOSER")
        } else {
            None
        };
        app.render_composer(
            self.app.clone(),
            self.suggestion_selection,
            &self.footer_scroll,
            &self.status_scroll,
            mode,
            cx,
        )
        .into_any_element()
    }
}
