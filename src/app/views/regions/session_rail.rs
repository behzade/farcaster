use std::cell::RefCell;

use gpui::{Context, IntoElement as _, ListAlignment, ListState, Pixels, Render, WeakEntity};

use super::super::{FarcasterApp, SessionRailKind};
use crate::app::ui::theme::THEME;

pub(crate) struct SessionRailView {
    app: WeakEntity<FarcasterApp>,
    list: ListState,
    rows: RefCell<Vec<String>>,
    width: Pixels,
    resize_start: Option<(Pixels, Pixels)>,
    shortcuts_visible: bool,
}

pub(crate) struct InactiveSessionRailView {
    app: WeakEntity<FarcasterApp>,
    kind: SessionRailKind,
    list: ListState,
    rows: RefCell<Vec<String>>,
}

fn session_list() -> ListState {
    ListState::new(0, ListAlignment::Top, THEME.layout.transcript_overdraw)
}

impl SessionRailView {
    pub(crate) fn new(app: WeakEntity<FarcasterApp>) -> Self {
        Self {
            app,
            list: session_list(),
            rows: RefCell::new(Vec::new()),
            width: THEME.layout.session_rail,
            resize_start: None,
            shortcuts_visible: false,
        }
    }

    pub(crate) fn width(&self) -> Pixels {
        self.width
    }

    pub(crate) fn shortcuts_visible(&self) -> bool {
        self.shortcuts_visible
    }

    pub(crate) fn set_shortcuts_visible(&mut self, visible: bool) -> bool {
        if self.shortcuts_visible == visible {
            return false;
        }
        self.shortcuts_visible = visible;
        true
    }

    pub(crate) fn begin_resize(&mut self, pointer_x: Pixels) {
        self.resize_start = Some((pointer_x, self.width));
    }

    pub(crate) fn update_resize(&mut self, pointer_x: Pixels) -> bool {
        let Some((start_x, start_width)) = self.resize_start else {
            return false;
        };
        let width = super::super::session_rail::clamped_session_rail_width(
            f32::from(start_width) + f32::from(pointer_x) - f32::from(start_x),
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
}

impl InactiveSessionRailView {
    pub(crate) fn new(app: WeakEntity<FarcasterApp>, kind: SessionRailKind) -> Self {
        Self {
            app,
            kind,
            list: session_list(),
            rows: RefCell::new(Vec::new()),
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
            .render_sessions(
                self.app.clone(),
                cx.has_active_drag(),
                self.list.clone(),
                &self.rows,
                self.shortcuts_visible,
            )
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
        app.read(cx).render_inactive_sessions(
            self.app.clone(),
            self.kind,
            self.list.clone(),
            &self.rows,
        )
    }
}
