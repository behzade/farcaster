use std::collections::{HashMap, HashSet};

use gpui::{Div, InteractiveElement as _, Stateful, Styled as _, WeakEntity, px};

use super::{
    FarcasterApp,
    drag::DraggedSession,
    groups::{SessionRailItem, SessionRailKind},
    rows::session_badge,
};
use crate::{app::ui::theme::THEME, sessions::SessionSummary};

pub(super) const INACTIVE_PREVIEW_LIMIT: usize = 5;
pub(super) const ARCHIVED_LEADING_GAP: f32 = 34.0;

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
