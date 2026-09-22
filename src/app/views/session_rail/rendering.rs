use std::collections::{HashMap, HashSet};

use gpui::{
    Bounds, Div, DragMoveEvent, FontWeight, InteractiveElement as _, ListState, Pixels, Point,
    Stateful, Styled as _, WeakEntity, div,
};

use super::{
    FarcasterApp,
    drag::DraggedSession,
    groups::{SessionRailItem, SessionRailKind},
};
use crate::{
    app::{
        session::status::resolved_session_status,
        ui::{
            primitives::{PanelSlot, ReorderPosition},
            theme::theme,
        },
    },
    sessions::SessionSummary,
};

/// The archive opens showing this much of itself; its separator can then be
/// dragged anywhere, exactly like the notification panel's.
pub(super) const ARCHIVED_OPEN_ROWS: usize = 5;

/// The rail's section headers — folders, and the panels that share the same
/// shape — are exactly as tall as the disclosure control they lead with, so the
/// arrow never carries empty box space above or below it.
pub(super) fn session_section_header() -> Div {
    div()
        .w_full()
        .h(theme().controls.icon_button)
        .flex_none()
        .flex()
        .items_center()
        .gap(theme().space.xs)
        .pr(theme().space.xs)
        .text_size(theme().type_scale.caption)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme().colors.muted)
}

fn session_list_end_target(
    viewport: Bounds<Pixels>,
    last_row: Option<(i64, Bounds<Pixels>)>,
    pointer: Point<Pixels>,
) -> Option<i64> {
    let (id, bounds) = last_row?;
    (viewport.contains(&pointer) && pointer.y >= bounds.bottom()).then_some(id)
}

pub(super) fn active_session_drop_target(
    section: Stateful<Div>,
    list: ListState,
    last_row: Option<(usize, i64)>,
    entity: WeakEntity<FarcasterApp>,
) -> Stateful<Div> {
    let end_target = move |drag: &DraggedSession, pointer| {
        session_list_end_target(
            list.viewport_bounds(),
            last_row.and_then(|(index, id)| list.bounds_for_item(index).map(|bounds| (id, bounds))),
            pointer,
        )
        .filter(|id| drag.can_drop_on(SessionRailKind::Project, *id))
    };
    let can_drop_target = end_target.clone();
    let move_target = end_target.clone();
    let move_entity = entity.clone();
    section
        .can_drop(move |value, window, _| {
            value.downcast_ref::<DraggedSession>().is_some_and(|drag| {
                drag.can_move_to(SessionRailKind::Project)
                    || can_drop_target(drag, window.mouse_position()).is_some()
            })
        })
        .on_drag_move(move |event: &DragMoveEvent<DraggedSession>, _, cx| {
            if let Some(target) = move_target(event.drag(cx), event.event.position) {
                let _ = move_entity.update(cx, |this, cx| {
                    this.update_session_drop_target(target, ReorderPosition::After, cx);
                });
            }
        })
        .on_drop(move |drag: &DraggedSession, window, cx| {
            cx.stop_propagation();
            let target = end_target(drag, window.mouse_position());
            let _ = entity.update(cx, |this, cx| {
                this.sessions.drop_target = target.map(|id| (id, ReorderPosition::After));
                this.complete_session_row_drop(drag, SessionRailKind::Project, window, cx);
            });
        })
}

pub(super) fn session_section_drop_target(
    section: Stateful<Div>,
    kind: SessionRailKind,
    entity: WeakEntity<FarcasterApp>,
) -> Stateful<Div> {
    section
        .can_drop(move |value, _, _| {
            value
                .downcast_ref::<DraggedSession>()
                .is_some_and(|drag| drag.can_move_to(kind))
        })
        .on_drop(move |drag: &DraggedSession, window, cx| {
            cx.stop_propagation();
            let _ = entity.update(cx, |this, cx| {
                this.complete_session_category_drop(drag, kind, window, cx);
            });
        })
}

pub(super) fn subagent_counts(sessions: &[SessionSummary]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for session in sessions {
        if let Some(parent) = &session.parent_session {
            *counts.entry(parent.clone()).or_default() += 1;
        }
    }
    counts
}

pub(super) fn inactive_session_badge(
    kind: SessionRailKind,
    item: &SessionRailItem,
    run_statuses: &HashMap<String, String>,
    live_root: Option<&str>,
    live_status: &str,
    waiting_roots: &HashSet<String>,
) -> Option<String> {
    if kind != SessionRailKind::Archived {
        return None;
    }
    let target = format!("session:{}", item.session.path.display());
    let status = resolved_session_status(
        &item.session,
        run_statuses.get(&target).map(String::as_str),
        live_root,
        live_status,
        waiting_roots.contains(&item.session.id),
    );
    (status != "Done").then_some(status)
}
/// The archive's body is flush, so a row is whole as soon as its own height
/// fits under the panel header.
fn archived_panel_base() -> Pixels {
    theme().controls.icon_button
}

/// The height the archive needs to show exactly this many rows.
pub(super) fn archived_panel_height(rows: usize) -> Pixels {
    archived_panel_base() + theme().controls.archived_preview_row * rows as f32
}

pub(super) fn archived_panel_slot(count: usize, collapsed: bool) -> PanelSlot {
    if collapsed {
        return PanelSlot::new(theme().controls.icon_button, theme().controls.icon_button);
    }
    PanelSlot::new(
        archived_panel_height(1),
        archived_panel_height(count.clamp(1, ARCHIVED_OPEN_ROWS)),
    )
}

pub(super) fn notification_panel_slot(collapsed: bool) -> PanelSlot {
    if collapsed {
        return PanelSlot::new(theme().controls.icon_button, theme().controls.icon_button);
    }
    PanelSlot::new(theme().layout.notice_panel_min, theme().layout.notice_panel)
}

pub(super) fn archived_panel_rows(height: Pixels) -> usize {
    ((height - archived_panel_base()) / theme().controls.archived_preview_row).max(0.0) as usize
}

#[cfg(test)]
#[path = "rendering_tests.rs"]
mod tests;
