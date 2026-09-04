use gpui::{Pixels, px};

use super::super::FarcasterApp;
use crate::app::ui::theme::THEME;

pub(super) fn clamped_run_panel_width(width: f32) -> Pixels {
    px(width.clamp(
        f32::from(THEME.layout.run_panel_min),
        f32::from(THEME.layout.run_panel_max),
    ))
}

impl FarcasterApp {
    pub(in crate::app::views) fn begin_run_panel_resize(
        &mut self,
        pointer_x: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        self.view.run_panel.resize_start = Some((pointer_x, self.view.run_panel.width));
        cx.notify();
    }

    pub(in crate::app::views) fn update_run_panel_resize(
        &mut self,
        pointer_x: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some((start_x, start_width)) = self.view.run_panel.resize_start else {
            return;
        };
        let width = clamped_run_panel_width(
            f32::from(start_width) + f32::from(start_x) - f32::from(pointer_x),
        );
        if width != self.view.run_panel.width {
            self.view.run_panel.width = width;
            cx.notify();
        }
    }

    pub(in crate::app::views) fn finish_run_panel_resize(&mut self, cx: &mut gpui::Context<Self>) {
        if self.view.run_panel.resize_start.take().is_some() {
            cx.notify();
        }
    }
}
