use std::time::{Duration, Instant};

use gpui::{Context, Pixels, Point};

use super::FarcasterApp;

#[derive(Clone, Copy)]
pub(super) enum Sidebar {
    Sessions,
    SourceControl,
}

pub(in crate::app) struct SidebarResize {
    sidebar: Sidebar,
    origin: Point<Pixels>,
    started: Instant,
    moved: bool,
}

impl SidebarResize {
    fn track(&mut self, position: Point<Pixels>) {
        let delta = position - self.origin;
        self.moved |= f32::from(delta.x).hypot(f32::from(delta.y)) >= 4.0;
    }

    fn is_click(&self, now: Instant) -> bool {
        !self.moved && now.duration_since(self.started) < Duration::from_millis(250)
    }
}

impl FarcasterApp {
    pub(super) fn begin_sidebar_resize(
        &mut self,
        sidebar: Sidebar,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.finish_resizes(cx);
        self.views.sidebar_resize = Some(SidebarResize {
            sidebar,
            origin: position,
            started: Instant::now(),
            moved: false,
        });
        match sidebar {
            Sidebar::Sessions => self.begin_session_rail_resize(position.x, cx),
            Sidebar::SourceControl => self.begin_run_panel_resize(position.x, cx),
        }
    }

    pub(super) fn update_sidebar_resize(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(resize) = self.views.sidebar_resize.as_mut() else {
            return;
        };
        resize.track(position);
        if !resize.moved {
            return;
        }
        match resize.sidebar {
            Sidebar::Sessions => self.update_session_rail_resize(position.x, cx),
            Sidebar::SourceControl => self.update_run_panel_resize(position.x, cx),
        }
    }

    pub(super) fn release_sidebar_resize(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let collapse = self.views.sidebar_resize.take().and_then(|mut resize| {
            resize.track(position);
            resize.is_click(Instant::now()).then_some(resize.sidebar)
        });
        self.finish_resizes(cx);
        match collapse {
            Some(Sidebar::Sessions) => self.toggle_session_rail(cx),
            Some(Sidebar::SourceControl) => self.toggle_run_panel(cx),
            None => {}
        }
    }
}

#[cfg(test)]
#[path = "sidebar_resize_tests.rs"]
mod tests;
