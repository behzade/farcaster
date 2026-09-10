use std::cell::RefCell;

use gpui::{Context, IntoElement as _, ListAlignment, ListState, Pixels, Render, WeakEntity};

use super::super::{FarcasterApp, SessionRailKind};
use crate::app::ui::theme::THEME;

pub(crate) struct SessionRailView {
    app: WeakEntity<FarcasterApp>,
    list: ListState,
    rows: RefCell<Vec<String>>,
    pub(crate) reveal: Option<String>,
    width: Pixels,
    resize_start: Option<(Pixels, Pixels)>,
}

pub(crate) struct InactiveSessionRailView {
    app: WeakEntity<FarcasterApp>,
    kind: SessionRailKind,
    list: ListState,
    rows: RefCell<Vec<String>>,
    pub(crate) reveal: Option<String>,
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
            reveal: None,
            width: THEME.layout.session_rail,
            resize_start: None,
        }
    }

    pub(crate) fn width(&self) -> Pixels {
        self.width
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
            list: session_list().with_uniform_item_height(THEME.layout.session_row_height),
            rows: RefCell::new(Vec::new()),
            reveal: None,
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
        let content = app
            .read(cx)
            .render_sessions(
                self.app.clone(),
                cx.has_active_drag(),
                self.list.clone(),
                &self.rows,
            )
            .into_any_element();
        reveal_session_row(&self.list, &self.rows, &mut self.reveal);
        content
    }
}

impl Render for InactiveSessionRailView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let _timing =
            crate::app::infrastructure::performance::Timing::new("render.inactive_session_sidebar");
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        let content = app.read(cx).render_inactive_sessions(
            self.app.clone(),
            self.kind,
            self.list.clone(),
            &self.rows,
        );
        reveal_session_row(&self.list, &self.rows, &mut self.reveal);
        content
    }
}

fn reveal_session_row(list: &ListState, rows: &RefCell<Vec<String>>, reveal: &mut Option<String>) {
    if let Some(key) = reveal.take()
        && let Some(index) = rows.borrow().iter().position(|row| row == &key)
    {
        list.scroll_to_reveal_item(index);
        // On first expansion there may be no viewport measurement yet.
        if list.logical_scroll_top().item_ix > index {
            list.scroll_to(gpui::ListOffset {
                item_ix: index,
                offset_in_item: gpui::px(0.0),
            });
        }
    }
}

#[cfg(test)]
#[path = "session_rail_tests.rs"]
mod tests;
