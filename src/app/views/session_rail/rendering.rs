use std::collections::{HashMap, HashSet};

use gpui::{
    Bounds, Div, DragMoveEvent, InteractiveElement as _, ListState, Pixels, Point, Stateful,
    Styled as _, WeakEntity, px,
};

use super::{
    FarcasterApp,
    drag::DraggedSession,
    groups::{SessionRailItem, SessionRailKind},
    rows::session_badge,
};
use crate::{
    app::ui::{primitives::ReorderPosition, theme::THEME},
    sessions::SessionSummary,
};

pub(super) const INACTIVE_PREVIEW_LIMIT: usize = 5;
pub(super) const ARCHIVED_LEADING_GAP: f32 = 34.0;

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
                this.session_drop_target = target.map(|id| (id, ReorderPosition::After));
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
    session_badge(
        item,
        run_statuses.get(&target).map(String::as_str),
        live_root,
        live_status,
        waiting_roots.contains(&item.session.id),
    )
}

pub(super) fn collapsed_inactive_rail_height(count: usize, leading_gap: bool) -> gpui::Pixels {
    let rows = THEME.controls.utility_row
        + THEME.controls.archived_preview_row * count.min(INACTIVE_PREVIEW_LIMIT);
    if leading_gap {
        rows + px(ARCHIVED_LEADING_GAP)
    } else {
        rows
    }
}

pub(super) fn inactive_rail_style(
    expanded: bool,
    count: usize,
    leading_gap: bool,
) -> gpui::StyleRefinement {
    if expanded {
        gpui::StyleRefinement::default().size_full().flex_1()
    } else {
        gpui::StyleRefinement::default()
            .w_full()
            .h(collapsed_inactive_rail_height(count, leading_gap))
            .min_h(gpui::relative(0.36))
            .flex_none()
    }
}

#[cfg(test)]
#[path = "rendering_tests.rs"]
mod tests;
