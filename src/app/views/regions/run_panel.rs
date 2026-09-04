use gpui::{Context, IntoElement as _, Pixels, Render, ScrollHandle, WeakEntity};

use super::super::FarcasterApp;
use crate::app::ui::theme::THEME;

pub(crate) struct RunPanelView {
    app: WeakEntity<FarcasterApp>,
    width: Pixels,
    resize_start: Option<(Pixels, Pixels)>,
    scroll: ScrollHandle,
    completed_agents_expanded: bool,
    limited_agents_expanded: bool,
}

impl RunPanelView {
    pub(crate) fn new(app: WeakEntity<FarcasterApp>) -> Self {
        Self {
            app,
            width: THEME.layout.run_panel,
            resize_start: None,
            scroll: ScrollHandle::new(),
            completed_agents_expanded: false,
            limited_agents_expanded: false,
        }
    }

    pub(crate) fn width(&self) -> Pixels {
        self.width
    }

    pub(crate) fn reset_scroll(&self) {
        self.scroll
            .set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
    }

    pub(crate) fn begin_resize(&mut self, pointer_x: Pixels) {
        self.resize_start = Some((pointer_x, self.width));
    }

    pub(crate) fn update_resize(&mut self, pointer_x: Pixels) -> bool {
        let Some((start_x, start_width)) = self.resize_start else {
            return false;
        };
        let width = super::super::run_panel::clamped_run_panel_width(
            f32::from(start_width) + f32::from(start_x) - f32::from(pointer_x),
        );
        if width == self.width {
            return false;
        }
        self.width = width;
        true
    }

    pub(crate) fn finish_resize(&mut self) -> bool {
        self.resize_start.take().is_some()
    }

    pub(crate) fn toggle_completed_agents(&mut self) {
        self.completed_agents_expanded = !self.completed_agents_expanded;
    }

    pub(crate) fn toggle_limited_agents(&mut self) {
        self.limited_agents_expanded = !self.limited_agents_expanded;
    }
}

impl Render for RunPanelView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let _timing = crate::app::infrastructure::performance::Timing::new("render.run_sidebar");
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        app.read(cx)
            .render_run_panel(
                self.app.clone(),
                cx.entity().downgrade(),
                &self.scroll,
                self.completed_agents_expanded,
                self.limited_agents_expanded,
            )
            .into_any_element()
    }
}
