use std::cell::RefCell;

use gpui::{
    Context, IntoElement as _, ListAlignment, ListState, Pixels, Render, ScrollStrategy,
    UniformListScrollHandle, WeakEntity,
};

#[cfg(test)]
use super::super::session_rail::session_row_height;
use super::super::{
    FarcasterApp, SessionRailKind,
    session_rail::{ArchivedRow, reconcile_archived_rows, remap_archived_scroll},
};
use crate::app::infrastructure::performance::{OperationKind, OperationTiming, Timing};
use crate::app::ui::theme::theme;
use crate::utility::persistent_vec::PersistentVec;

pub(crate) struct SessionRailView {
    app: WeakEntity<FarcasterApp>,
    list: ListState,
    rows: RefCell<PersistentVec<String>>,
    pub(crate) reveal: Option<SessionReveal>,
    width: Pixels,
    resize_start: Option<(Pixels, Pixels)>,
    grouped: bool,
    prepared: Option<super::super::session_rail::PreparedActiveRows>,
    #[cfg(test)]
    render_count: usize,
}

pub(crate) enum SessionReveal {
    SessionID(String),
    Index(usize),
}

impl SessionReveal {
    fn into_index(self, rows: &PersistentVec<String>) -> Option<usize> {
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
    rows: PersistentVec<ArchivedRow>,
    pub(crate) reveal: Option<SessionReveal>,
    #[cfg(test)]
    pub(in crate::app) preparations: usize,
    #[cfg(test)]
    pub(in crate::app) renders: usize,
    prepared: bool,
}

fn session_list() -> ListState {
    ListState::new(0, ListAlignment::Top, theme().layout.transcript_overdraw)
}

impl SessionRailView {
    pub(crate) fn new(app: WeakEntity<FarcasterApp>) -> Self {
        Self {
            app,
            list: session_list(),
            rows: RefCell::default(),
            reveal: None,
            width: theme().layout.session_rail,
            resize_start: None,
            grouped: false,
            prepared: None,
            #[cfg(test)]
            render_count: 0,
        }
    }

    pub(in crate::app::views) fn prepared_rows(
        &mut self,
        app: &FarcasterApp,
    ) -> super::super::session_rail::PreparedActiveRows {
        let grouped = app.settings.group_sessions_by_project;
        if self.grouped != grouped {
            self.list.reset(0);
            self.rows.borrow_mut().clear();
            self.grouped = grouped;
            self.invalidate_rows();
        }
        self.prepared
            .get_or_insert_with(|| app.prepare_active_rows(&self.list, &self.rows))
            .clone()
    }

    pub(in crate::app) fn invalidate_rows(&mut self) {
        self.prepared = None;
    }

    pub(in crate::app) fn patch_archive(
        &mut self,
        app: &FarcasterApp,
        key: Option<&crate::sessions::SessionKey>,
        draft: Option<&str>,
    ) {
        if let Some(prepared) = self.prepared.as_mut()
            && !prepared.patch_archive(app, key, draft, &self.list, &self.rows)
        {
            self.invalidate_rows();
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
    pub(in crate::app::views) fn sorted_rows(
        &mut self,
        app: &FarcasterApp,
    ) -> PersistentVec<ArchivedRow> {
        if !self.prepared {
            #[cfg(test)]
            {
                self.preparations += 1;
            }
            let next = app.prepare_archived_rows();
            reconcile_archived_rows(&self.list, &self.rows, &next);
            self.rows = next;
            self.prepared = true;
        }
        self.rows.clone()
    }

    pub(in crate::app) fn invalidate_rows(&mut self) {
        self.prepared = false;
    }

    pub(in crate::app::views) fn has_sorted_rows(&self) -> bool {
        self.prepared
    }

    pub(in crate::app::views) fn set_membership(&mut self, row: ArchivedRow, archived: bool) {
        let index = self.rows.partition_point(|item| item.order > row.order);
        let matching = self.rows.get(index).filter(|item| item.order == row.order);
        // Stable recency ties between unassigned IDs follow source catalog order.
        // Resolving a new tied row or a stale key requires a fresh ordering.
        if archived && matching.is_some()
            || !archived && matching.is_none_or(|item| item.item != row.item)
        {
            self.invalidate_rows();
            return;
        }
        let old_len = self.rows.len();
        if archived {
            self.rows.insert(index, row);
        } else {
            self.rows.remove(index);
        }
        remap_archived_scroll(&self.list, self.rows.len(), |old| {
            if old >= old_len || (!archived && old == index) {
                None
            } else if old < index {
                Some(old)
            } else if archived {
                Some(old + 1)
            } else {
                Some(old - 1)
            }
        });
    }

    pub(crate) fn new(app: WeakEntity<FarcasterApp>, kind: SessionRailKind) -> Self {
        Self {
            app,
            kind,
            list: UniformListScrollHandle::new(),
            rows: PersistentVec::default(),
            reveal: None,
            #[cfg(test)]
            preparations: 0,
            #[cfg(test)]
            renders: 0,
            prepared: false,
        }
    }
}

impl Render for SessionRailView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        let _timing = Timing::new("render.session_sidebar");
        let _operation = OperationTiming::new(OperationKind::SessionSidebar, 1);
        #[cfg(test)]
        {
            self.render_count += 1;
        }
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        let prepared = self.prepared_rows(app.read(cx));
        let content = app
            .read(cx)
            .render_sessions(
                self.app.clone(),
                cx.has_active_drag(),
                window.is_a11y_active(),
                self.list.clone(),
                prepared,
            )
            .into_any_element();
        reveal_session_row(&self.list, &self.rows, &mut self.reveal);
        content
    }
}

fn reveal_archived_session_row(
    list: &UniformListScrollHandle,
    len: usize,
    lookup: impl FnOnce(&str) -> Option<usize>,
    reveal: &mut Option<SessionReveal>,
) {
    let Some(index) = reveal.take().and_then(|target| match target {
        SessionReveal::Index(index) => (index < len).then_some(index),
        SessionReveal::SessionID(id) => lookup(&id),
    }) else {
        return;
    };

    list.scroll_to_item(index, ScrollStrategy::Nearest);
}

impl Render for InactiveSessionRailView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let _timing = Timing::new("render.inactive_session_sidebar");
        #[cfg(test)]
        {
            self.renders += 1;
        }
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        let app = app.read(cx);
        let rows = self.sorted_rows(app);
        let content =
            app.render_inactive_sessions(self.app.clone(), self.kind, self.list.clone(), rows);
        reveal_archived_session_row(
            &self.list,
            self.rows.len(),
            |id| self.rows.position(|row| row.identity == id),
            &mut self.reveal,
        );
        content
    }
}

fn reveal_session_row(
    list: &ListState,
    rows: &RefCell<PersistentVec<String>>,
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
