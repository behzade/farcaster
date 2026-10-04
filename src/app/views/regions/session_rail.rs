use std::cell::RefCell;

use gpui::{
    Context, IntoElement as _, ListAlignment, ListState, Pixels, Render, ScrollStrategy,
    UniformListScrollHandle, WeakEntity,
};

#[cfg(test)]
use super::super::session_rail::session_row_height;
use super::super::{FarcasterApp, SessionRailKind};
use crate::app::infrastructure::performance::{OperationKind, OperationTiming, Timing};
use crate::app::ui::theme::theme;

pub(crate) struct SessionRailView {
    app: WeakEntity<FarcasterApp>,
    list: ListState,
    rows: RefCell<Vec<String>>,
    pub(crate) reveal: Option<SessionReveal>,
    width: Pixels,
    resize_start: Option<(Pixels, Pixels)>,
    grouped: bool,
    #[cfg(test)]
    render_count: usize,
}

pub(crate) enum SessionReveal {
    SessionID(String),
    Index(usize),
}

impl SessionReveal {
    fn into_index(self, rows: &[String]) -> Option<usize> {
        match self {
            Self::SessionID(key) => rows.iter().position(|row| row == &key),
            Self::Index(index) => (index < rows.len()).then_some(index),
        }
    }
}

pub(crate) struct InactiveSessionRailView {
    app: WeakEntity<FarcasterApp>,
    kind: SessionRailKind,
    list: UniformListScrollHandle,
    rows: RefCell<Vec<String>>,
    pub(crate) reveal: Option<SessionReveal>,
}

fn session_list() -> ListState {
    ListState::new(0, ListAlignment::Top, theme().layout.transcript_overdraw)
}

impl SessionRailView {
    pub(crate) fn new(app: WeakEntity<FarcasterApp>) -> Self {
        Self {
            app,
            list: session_list(),
            rows: RefCell::new(Vec::new()),
            reveal: None,
            width: theme().layout.session_rail,
            resize_start: None,
            grouped: false,
            #[cfg(test)]
            render_count: 0,
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
            list: UniformListScrollHandle::new(),
            rows: RefCell::new(Vec::new()),
            reveal: None,
        }
    }
}

impl Render for SessionRailView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let _timing = Timing::new("render.session_sidebar");
        let _operation = OperationTiming::new(OperationKind::SessionSidebar, 1);
        #[cfg(test)]
        {
            self.render_count += 1;
        }
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        let grouped = app.read(cx).settings.group_sessions_by_project;
        if self.grouped != grouped {
            self.list.reset(0);
            self.rows.borrow_mut().clear();
            self.grouped = grouped;
        }
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

fn reveal_archived_session_row(
    list: &UniformListScrollHandle,
    rows: &RefCell<Vec<String>>,
    reveal: &mut Option<SessionReveal>,
) {
    let Some(index) = reveal
        .take()
        .and_then(|target| target.into_index(&rows.borrow()))
    else {
        return;
    };

    list.scroll_to_item(index, ScrollStrategy::Nearest);
}

impl Render for InactiveSessionRailView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let _timing = Timing::new("render.inactive_session_sidebar");
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        let content = app.read(cx).render_inactive_sessions(
            self.app.clone(),
            self.kind,
            self.list.clone(),
            &self.rows,
        );
        reveal_archived_session_row(&self.list, &self.rows, &mut self.reveal);
        content
    }
}

fn reveal_session_row(
    list: &ListState,
    rows: &RefCell<Vec<String>>,
    reveal: &mut Option<SessionReveal>,
) {
    let Some(index) = reveal
        .take()
        .and_then(|target| target.into_index(&rows.borrow()))
    else {
        return;
    };
    list.scroll_to_reveal_item(index);
    if list.logical_scroll_top().item_ix >= index {
        list.scroll_to(gpui::ListOffset {
            item_ix: index,
            offset_in_item: gpui::px(0.0),
        });
    }
}

#[cfg(test)]
#[path = "session_rail_tests.rs"]
mod tests;
