use gpui::{Context, IntoElement as _, Render, ScrollHandle, WeakEntity};

use super::super::FarcasterApp;

pub(crate) struct ComposerView {
    app: WeakEntity<FarcasterApp>,
    suggestion_selection: usize,
    completions_dismissed: bool,
    suggestion_scroll: ScrollHandle,
    footer_scroll: ScrollHandle,
    status_scroll: ScrollHandle,
}

impl ComposerView {
    pub(crate) fn new(app: WeakEntity<FarcasterApp>) -> Self {
        Self {
            app,
            suggestion_selection: 0,
            completions_dismissed: false,
            suggestion_scroll: ScrollHandle::new(),
            footer_scroll: ScrollHandle::new(),
            status_scroll: ScrollHandle::new(),
        }
    }

    pub(crate) fn suggestion_selection(&self) -> usize {
        self.suggestion_selection
    }

    pub(crate) fn reset_suggestion_selection(&mut self) {
        self.suggestion_selection = 0;
        self.completions_dismissed = false;
        self.suggestion_scroll.scroll_to_item(0);
    }

    pub(crate) fn completions_dismissed(&self) -> bool {
        self.completions_dismissed
    }

    pub(crate) fn dismiss_completions(&mut self, cx: &mut Context<Self>) {
        self.completions_dismissed = true;
        cx.notify();
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
        self.suggestion_scroll
            .scroll_to_item(self.suggestion_selection);
        cx.notify();
        true
    }

    pub(crate) fn select_next_suggestion(&mut self, count: usize, cx: &mut Context<Self>) -> bool {
        if count == 0 {
            return false;
        }
        self.suggestion_selection = (self.suggestion_selection + 1) % count;
        self.suggestion_scroll
            .scroll_to_item(self.suggestion_selection);
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
        app.render_composer(
            self.app.clone(),
            self.suggestion_selection,
            self.completions_dismissed,
            &self.suggestion_scroll,
            &self.footer_scroll,
            &self.status_scroll,
            app.navigation.chat.activation.hint(),
            app.composer_region_focused(window, cx),
            cx,
        )
        .into_any_element()
    }
}
