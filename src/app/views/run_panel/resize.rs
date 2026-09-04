use gpui::{Pixels, px};

use super::super::FarcasterApp;
use crate::app::ui::theme::THEME;

pub(in crate::app::views) fn clamped_run_panel_width(width: f32) -> Pixels {
    px(width.clamp(
        f32::from(THEME.layout.run_panel_min),
        f32::from(THEME.layout.run_panel_max),
    ))
}

impl FarcasterApp {
    pub(in crate::app) fn reset_run_panel_scroll(&mut self, cx: &mut gpui::Context<Self>) {
        self.run_panel_view
            .update(cx, |view, _| view.reset_scroll());
    }

    pub(in crate::app::views) fn begin_run_panel_resize(
        &mut self,
        pointer_x: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        self.run_panel_view
            .update(cx, |view, _| view.begin_resize(pointer_x));
        cx.notify();
    }

    pub(in crate::app::views) fn update_run_panel_resize(
        &mut self,
        pointer_x: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        let changed = self.run_panel_view.update(cx, |view, cx| {
            let changed = view.update_resize(pointer_x);
            if changed {
                cx.notify();
            }
            changed
        });
        if changed {
            cx.notify();
        }
    }

    pub(in crate::app::views) fn finish_run_panel_resize(&mut self, cx: &mut gpui::Context<Self>) {
        if self
            .run_panel_view
            .update(cx, |view, _| view.finish_resize())
        {
            cx.notify();
        }
    }
}
