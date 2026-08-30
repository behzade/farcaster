use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use gpui::{
    Anchor, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Stateful,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, list,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
};

#[cfg(test)]
use super::session_rows::{session_accessible_label, status_visual};
use super::{
    super::FarcasterApp,
    session_groups::{
        ActiveSessionItem, SessionRailItem, SessionRailKind, merge_visible_session_order,
        reordered_session_ids, roots_waiting_for_descendants, session_rail_lists,
    },
    session_rows::{
        DraggedSession, draft_session_row, project_label, session_badge, session_row,
        session_row_with_height,
    },
};
use crate::{
    app::{PickerScope, ProjectPickerIntent},
    assets::AppIcon,
    primitives::{
        AppIconSize, ButtonTone, FeedbackTone, ReorderPosition, app_icon, disclosure_button,
        dropdown_button, feedback, icon_button,
    },
    projects::{self, DraftSession},
    sessions::{SessionSummary, root_session_for_path},
    theme::THEME,
};

const INACTIVE_PREVIEW_LIMIT: usize = 3;

fn session_section_drop_target(
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

fn subagent_counts(sessions: &[SessionSummary]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for session in sessions {
        if let Some(parent) = &session.parent_session {
            *counts.entry(parent.clone()).or_default() += 1;
        }
    }
    counts
}

fn inactive_session_badge(
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

fn collapsed_inactive_rail_height(count: usize, leading_gap: bool) -> gpui::Pixels {
    let rows = THEME.controls.utility_row
        + THEME.controls.archived_preview_row * count.min(INACTIVE_PREVIEW_LIMIT);
    if leading_gap {
        rows + THEME.space.md
    } else {
        rows
    }
}

fn inactive_rail_style(expanded: bool, count: usize, leading_gap: bool) -> gpui::StyleRefinement {
    if expanded {
        gpui::StyleRefinement::default().size_full().flex_1()
    } else {
        gpui::StyleRefinement::default()
            .w_full()
            .h(collapsed_inactive_rail_height(count, leading_gap))
            .flex_none()
    }
}

impl FarcasterApp {
    pub(super) fn render_sessions(
        &self,
        entity: WeakEntity<Self>,
        session_drag_active: bool,
    ) -> impl IntoElement {
        let new_entity = entity.clone();
        let actions_entity = entity.clone();
        let cancel_drop_entity = entity.clone();
        let cancel_drop_out_entity = entity.clone();
        let active_drop_entity = entity.clone();
        let search_focus = self.search_focus.clone();
        let selected_root =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .map(|session| session.id.clone());
        let live_root =
            root_session_for_path(&self.sessions, self.snapshot.live_session.as_deref())
                .map(|session| session.id.clone());
        let waiting_roots = roots_waiting_for_descendants(&self.all_sessions);
        let lists = session_rail_lists(
            &self.sessions,
            &self.drafts,
            self.session_project_filter.as_deref(),
            &self.session_order,
        );
        let counts = subagent_counts(&self.all_sessions);
        let active_entry_count = lists.active.len();
        let archived_entry_count = lists.archived.len();
        let active_rows = lists.active;
        let session_shortcuts = visible_session_shortcuts(&active_rows);
        reconcile_list_rows(
            &self.session_list,
            &self.session_list_rows,
            active_rows.iter().map(active_item_identity).collect(),
        );

        let selected_draft = self.selected_draft.clone();
        let submitted_drafts = self.submitted_drafts.clone();
        let active_selected_root = selected_root.clone();
        let active_live_root = live_root.clone();
        let active_live_status = self.snapshot.live_status.clone();
        let active_run_statuses = self.run_statuses.clone();
        let active_waiting_roots = waiting_roots.clone();
        let active_row_entity = entity.clone();
        let active_editing_path = self
            .editing_session_title
            .as_ref()
            .map(|edit| edit.path.clone());
        let active_title_input = self.session_title_input.clone();
        let active_drop_target = self.session_drop_target;
        let shortcuts_visible = self.session_shortcuts_visible;
        let active_list = list(
            self.session_list.clone(),
            move |index, _, _| match active_rows.get(index) {
                Some(ActiveSessionItem::Draft(draft)) => {
                    let selected = selected_draft.as_deref() == Some(draft.id.as_str());
                    let status = crate::app::drafts::resolved_draft_status(
                        &draft.id,
                        &submitted_drafts,
                        &active_run_statuses,
                    );
                    let shortcut = shortcuts_visible
                        .then(|| session_shortcuts.get(&draft.app_session_id).copied())
                        .flatten();
                    let drop_position = active_drop_target
                        .filter(|(target, _)| *target == draft.app_session_id)
                        .map(|(_, position)| position);
                    draft_session_row(
                        draft,
                        selected,
                        &status,
                        shortcut,
                        drop_position,
                        active_row_entity.clone(),
                    )
                }
                Some(ActiveSessionItem::Session(item)) => {
                    let selected =
                        active_selected_root.as_deref() == Some(item.session.id.as_str());
                    let target = format!("session:{}", item.session.path.display());
                    let badge = session_badge(
                        item,
                        active_run_statuses.get(&target).map(String::as_str),
                        active_live_root.as_deref(),
                        &active_live_status,
                        active_waiting_roots.contains(&item.session.id),
                    );
                    let shortcut = shortcuts_visible
                        .then(|| session_shortcuts.get(&item.session.app_session_id).copied())
                        .flatten();
                    let editing =
                        active_editing_path.as_deref() == Some(item.session.path.as_path());
                    let drop_position = active_drop_target
                        .filter(|(target, _)| *target == item.session.app_session_id)
                        .map(|(_, position)| position);
                    session_row(
                        item,
                        selected,
                        badge,
                        shortcut,
                        drop_position,
                        true,
                        editing.then(|| active_title_input.clone()),
                        counts.get(item.session.id.as_str()).copied().unwrap_or(0),
                        active_row_entity.clone(),
                    )
                }
                None => div().into_any_element(),
            },
        )
        .size_full();

        let archived_expanded =
            !session_drag_active && self.archived_sessions_expanded && archived_entry_count > 0;
        let archived_session_rail_style =
            inactive_rail_style(archived_expanded, archived_entry_count, true);
        let archived_session_rail = self
            .archived_session_rail_view
            .clone()
            .cached(archived_session_rail_style);
        let projects = self.available_projects();
        let project_filter_entity = entity.clone();
        let filter_label = self
            .session_project_filter
            .as_deref()
            .map(project_label)
            .unwrap_or_else(|| "All".into());

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(THEME.colors.panel)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(THEME.space.xs)
                    .p(THEME.space.sm)
                    .border_b(THEME.border)
                    .border_color(THEME.colors.border)
                    .child(
                        div()
                            .h(THEME.controls.utility_row)
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(self.render_surface_switcher(entity.clone()))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(THEME.space.xs)
                                    .child(icon_button(
                                        "session-actions",
                                        AppIcon::List,
                                        "Actions",
                                        ButtonTone::Quiet,
                                        move |window, cx| {
                                            let _ = actions_entity.update(cx, |this, cx| {
                                                this.open_picker(PickerScope::Actions, window, cx);
                                            });
                                        },
                                    ))
                                    .child(icon_button(
                                        "new-session",
                                        AppIcon::Plus,
                                        "New session",
                                        ButtonTone::Quiet,
                                        move |window, cx| {
                                            let _ = new_entity.update(cx, |this, cx| {
                                                this.open_picker(
                                                    PickerScope::Projects(
                                                        ProjectPickerIntent::NewSession,
                                                    ),
                                                    window,
                                                    cx,
                                                );
                                            });
                                        },
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .id("session-search-surface")
                            .h(THEME.controls.utility_row)
                            .flex()
                            .items_center()
                            .gap(THEME.space.xs)
                            .pl(THEME.space.sm)
                            .rounded(THEME.radius)
                            .bg(THEME.colors.surface)
                            .text_color(THEME.colors.muted)
                            .on_click(move |_, window, cx| search_focus.focus(window, cx))
                            .child(app_icon(AppIcon::MagnifyingGlass, AppIconSize::Prominent))
                            .child(
                                Input::new(&self.search)
                                    .flex_1()
                                    .min_w_0()
                                    .appearance(false),
                            )
                            .child(
                                dropdown_button(
                                    "project-filter",
                                    filter_label,
                                    ButtonTone::Quiet,
                                    true,
                                )
                                .flex_none()
                                .dropdown_menu_with_anchor(
                                    Anchor::TopRight,
                                    move |menu, _, _| {
                                        let all_entity = project_filter_entity.clone();
                                        let mut menu = menu
                                            .min_w(px(220.0))
                                            .max_h(px(420.0))
                                            .label("Projects")
                                            .item(PopupMenuItem::new("All").on_click(
                                                move |_, _, cx| {
                                                    let _ = all_entity.update(cx, |this, cx| {
                                                        this.set_session_project_filter(None, cx);
                                                    });
                                                },
                                            ));
                                        for project in &projects {
                                            let target = project.clone();
                                            let filter_entity = project_filter_entity.clone();
                                            menu = menu.item(
                                                PopupMenuItem::new(project_label(project))
                                                    .on_click(move |_, _, cx| {
                                                        let _ =
                                                            filter_entity.update(cx, |this, cx| {
                                                                this.set_session_project_filter(
                                                                    Some(target.clone()),
                                                                    cx,
                                                                );
                                                            });
                                                    }),
                                            );
                                        }
                                        menu
                                    },
                                ),
                            ),
                    ),
            )
            .when_some(self.sessions_error.clone(), |rail, error| {
                rail.child(feedback("sessions-error", error, FeedbackTone::Error))
            })
            .child(
                div()
                    .id("session-list-scroll")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_y_hidden()
                    .on_mouse_up(gpui::MouseButton::Left, move |_, _, cx| {
                        let _ = cancel_drop_entity
                            .update(cx, |this, cx| this.clear_session_drop_target(cx));
                    })
                    .on_mouse_up_out(gpui::MouseButton::Left, move |_, _, cx| {
                        let _ = cancel_drop_out_entity
                            .update(cx, |this, cx| this.clear_session_drop_target(cx));
                    })
                    .when(!archived_expanded, |lists| {
                        lists.child(session_section_drop_target(
                            div()
                                .id("active-session-drop-area")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_hidden()
                                .child(active_list),
                            SessionRailKind::Project,
                            active_drop_entity,
                        ))
                    })
                    .when(archived_entry_count > 0, |lists| {
                        lists.child(archived_session_rail)
                    }),
            )
            .when(
                active_entry_count == 0
                    && archived_entry_count == 0
                    && self.sessions_error.is_none(),
                |rail| {
                    rail.child(
                        div()
                            .px(THEME.space.md)
                            .py(THEME.space.sm)
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child("No matching sessions"),
                    )
                },
            )
            .into_any_element()
    }

    pub(super) fn render_inactive_sessions(
        &self,
        entity: WeakEntity<Self>,
        kind: SessionRailKind,
    ) -> gpui::AnyElement {
        debug_assert!(kind != SessionRailKind::Project);
        let selected_root =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .map(|session| session.id.clone());
        let live_root =
            root_session_for_path(&self.sessions, self.snapshot.live_session.as_deref())
                .map(|session| session.id.clone());
        let waiting_roots = roots_waiting_for_descendants(&self.all_sessions);
        let lists = session_rail_lists(
            &self.sessions,
            &self.drafts,
            self.session_project_filter.as_deref(),
            &self.session_order,
        );
        let (rows, list_state, list_rows, expanded) = match kind {
            SessionRailKind::Archived => (
                lists.archived,
                self.archived_session_list.clone(),
                &self.archived_session_list_rows,
                self.archived_sessions_expanded,
            ),
            SessionRailKind::Project => unreachable!("active sessions use the main rail"),
        };
        let count = rows.len();
        let expanded = expanded && count > 0;
        let counts = subagent_counts(&self.all_sessions);
        reconcile_list_rows(
            &list_state,
            list_rows,
            rows.iter().map(session_item_identity).collect(),
        );

        let preview_elements = rows
            .iter()
            .take(INACTIVE_PREVIEW_LIMIT)
            .map(|item| {
                let selected = selected_root.as_deref() == Some(item.session.id.as_str());
                let badge = inactive_session_badge(
                    kind,
                    item,
                    &self.run_statuses,
                    live_root.as_deref(),
                    &self.snapshot.live_status,
                    &waiting_roots,
                );
                let editing = self
                    .editing_session_title
                    .as_ref()
                    .is_some_and(|edit| edit.path == item.session.path);
                session_row_with_height(
                    item,
                    selected,
                    badge,
                    None,
                    None,
                    true,
                    editing.then(|| self.session_title_input.clone()),
                    counts.get(item.session.id.as_str()).copied().unwrap_or(0),
                    THEME.controls.archived_preview_row,
                    entity.clone(),
                )
            })
            .collect::<Vec<_>>();
        let row_entity = entity.clone();
        let editing_path = self
            .editing_session_title
            .as_ref()
            .map(|edit| edit.path.clone());
        let title_input = self.session_title_input.clone();
        let live_status = self.snapshot.live_status.clone();
        let run_statuses = self.run_statuses.clone();
        let rows_list = list(list_state, move |index, _, _| match rows.get(index) {
            Some(item) => {
                let selected = selected_root.as_deref() == Some(item.session.id.as_str());
                let badge = inactive_session_badge(
                    kind,
                    item,
                    &run_statuses,
                    live_root.as_deref(),
                    &live_status,
                    &waiting_roots,
                );
                let editing = editing_path.as_deref() == Some(item.session.path.as_path());
                session_row(
                    item,
                    selected,
                    badge,
                    None,
                    None,
                    true,
                    editing.then(|| title_input.clone()),
                    counts.get(item.session.id.as_str()).copied().unwrap_or(0),
                    row_entity.clone(),
                )
            }
            None => div().into_any_element(),
        })
        .size_full();

        let drop_entity = entity.clone();
        let toggle_entity = entity;
        let section = div()
            .id("archived-sessions")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .when(!expanded, |section| section.pt(THEME.space.md))
            .child(
                div()
                    .h(THEME.controls.utility_row)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(THEME.space.md)
                    .border_t(THEME.border)
                    .border_color(THEME.colors.border)
                    .text_size(THEME.type_scale.caption)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(THEME.colors.muted)
                    .child(format!("Archived · {count}"))
                    .child(disclosure_button(
                        "toggle-archived-sessions",
                        expanded,
                        "Archived sessions",
                        move |_, cx| {
                            let _ = toggle_entity.update(cx, |this, cx| {
                                this.archived_sessions_expanded = !this.archived_sessions_expanded;
                                this.notify_session_rail(cx);
                            });
                        },
                    )),
            )
            .when(!expanded, |section| section.children(preview_elements))
            .when(expanded, |section| {
                section.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_hidden()
                        .child(rows_list),
                )
            });
        session_section_drop_target(section, kind, drop_entity).into_any_element()
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
enum VisibleSessionTarget {
    Draft(DraftSession),
    Persisted(SessionSummary),
}

impl VisibleSessionTarget {
    fn app_session_id(&self) -> i64 {
        match self {
            Self::Draft(draft) => draft.app_session_id,
            Self::Persisted(session) => session.app_session_id,
        }
    }
}

fn active_item_identity(item: &ActiveSessionItem) -> String {
    match item {
        ActiveSessionItem::Draft(draft) => format!("draft:{}", draft.id),
        ActiveSessionItem::Session(item) => format!("session:{}", item.session.id),
    }
}

fn session_item_identity(item: &SessionRailItem) -> String {
    format!("session:{}", item.session.id)
}

fn reconcile_list_rows(
    list: &gpui::ListState,
    current: &std::cell::RefCell<Vec<String>>,
    next: Vec<String>,
) {
    let mut current = current.borrow_mut();
    if let Some((range, count)) = minimal_row_splice(&current, &next) {
        list.splice(range, count);
        *current = next;
    }
}

fn minimal_row_splice<T: Eq>(current: &[T], next: &[T]) -> Option<(std::ops::Range<usize>, usize)> {
    let prefix = current
        .iter()
        .zip(next)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = current[prefix..]
        .iter()
        .rev()
        .zip(next[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let old_end = current.len().saturating_sub(suffix);
    let replacement_count = next.len().saturating_sub(prefix + suffix);
    (prefix != old_end || replacement_count != 0).then_some((prefix..old_end, replacement_count))
}

fn first_unsubmitted_draft(rows: &[ActiveSessionItem]) -> Option<&DraftSession> {
    rows.iter().find_map(|row| match row {
        ActiveSessionItem::Draft(draft) if !draft.submitted => Some(draft),
        ActiveSessionItem::Draft(_) | ActiveSessionItem::Session(_) => None,
    })
}

fn visible_session_shortcuts(rows: &[ActiveSessionItem]) -> HashMap<i64, u8> {
    let mut shortcuts = rows
        .iter()
        .filter_map(|row| match row {
            ActiveSessionItem::Draft(draft) if draft.submitted => Some(draft.app_session_id),
            ActiveSessionItem::Session(item) => Some(item.session.app_session_id),
            ActiveSessionItem::Draft(_) => None,
        })
        .filter(|id| *id > 0)
        .take(9)
        .enumerate()
        .map(|(index, id)| (id, (index + 1) as u8))
        .collect::<HashMap<_, _>>();
    if let Some(draft) = first_unsubmitted_draft(rows)
        && draft.app_session_id > 0
    {
        shortcuts.insert(draft.app_session_id, 0);
    }
    shortcuts
}

impl FarcasterApp {
    pub(super) fn switch_to_first_unsubmitted_draft(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let lists = session_rail_lists(
            &self.sessions,
            &self.drafts,
            self.session_project_filter.as_deref(),
            &self.session_order,
        );
        if let Some(draft) = first_unsubmitted_draft(&lists.active).cloned() {
            self.select_visible_session(VisibleSessionTarget::Draft(draft), window, cx);
        }
    }

    pub(super) fn switch_to_session_number(
        &mut self,
        number: usize,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(target) = self
            .visible_session_targets()
            .get(number.saturating_sub(1))
            .cloned()
        {
            self.select_visible_session(target, window, cx);
        }
    }

    pub(super) fn switch_relative_session(
        &mut self,
        direction: isize,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let sessions = self.visible_session_targets();
        let selected_id = self
            .selected_draft
            .as_deref()
            .and_then(|id| self.drafts.iter().find(|draft| draft.id == id))
            .map(|draft| draft.app_session_id)
            .or_else(|| {
                root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                    .map(|session| session.app_session_id)
            });
        let selected = selected_id.and_then(|selected_id| {
            sessions
                .iter()
                .position(|session| session.app_session_id() == selected_id)
        });
        let Some(current) = selected else { return };
        let next = current as isize + direction;
        if next >= 0
            && let Some(target) = sessions.get(next as usize).cloned()
        {
            self.select_visible_session(target, window, cx);
        }
    }

    fn select_visible_session(
        &mut self,
        target: VisibleSessionTarget,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.workspace_switch_blocked() {
            return;
        }
        match target {
            VisibleSessionTarget::Draft(draft) => {
                self.resume_draft(draft.id, draft.project, window, cx);
            }
            VisibleSessionTarget::Persisted(session) => {
                self.select_session(session.path, session.project, window, cx);
            }
        }
    }

    fn visible_session_targets(&self) -> Vec<VisibleSessionTarget> {
        session_rail_lists(
            &self.sessions,
            &self.drafts,
            self.session_project_filter.as_deref(),
            &self.session_order,
        )
        .active
        .into_iter()
        .filter_map(|row| match row {
            ActiveSessionItem::Draft(draft) if draft.submitted => {
                Some(VisibleSessionTarget::Draft(draft))
            }
            ActiveSessionItem::Session(item) => Some(VisibleSessionTarget::Persisted(item.session)),
            ActiveSessionItem::Draft(_) => None,
        })
        .collect()
    }

    pub(super) fn begin_session_drag(&mut self, cx: &mut gpui::Context<Self>) {
        self.session_drop_target = None;
        self.notify_session_rail(cx);
    }

    pub(super) fn update_session_drop_target(
        &mut self,
        target: i64,
        position: ReorderPosition,
        cx: &mut gpui::Context<Self>,
    ) {
        let next = Some((target, position));
        if self.session_drop_target != next {
            self.session_drop_target = next;
            self.notify_session_rail(cx);
        }
    }

    pub(super) fn clear_session_drop_target(&mut self, cx: &mut gpui::Context<Self>) {
        self.session_drop_target = None;
        self.notify_session_rail(cx);
    }

    pub(super) fn complete_session_row_drop(
        &mut self,
        drag: &DraggedSession,
        target_kind: SessionRailKind,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if drag.can_move_to(target_kind) {
            self.complete_session_category_drop(drag, target_kind, window, cx);
            return;
        }
        if target_kind != SessionRailKind::Project {
            self.clear_session_drop_target(cx);
            return;
        }
        let Some((target, position)) = self.session_drop_target.take() else {
            self.clear_session_drop_target(cx);
            return;
        };
        let visible = session_rail_lists(
            &self.sessions,
            &self.drafts,
            self.session_project_filter.as_deref(),
            &self.session_order,
        )
        .active
        .iter()
        .map(ActiveSessionItem::app_session_id)
        .collect::<Vec<_>>();
        if let Some(order) = reordered_session_ids(&visible, drag.app_session_id, target, position)
        {
            let all = session_rail_lists(&self.sessions, &self.drafts, None, &self.session_order)
                .active
                .iter()
                .map(ActiveSessionItem::app_session_id)
                .collect::<Vec<_>>();
            let active_order = merge_visible_session_order(&all, &order);
            let active_ids = all.into_iter().collect::<HashSet<_>>();
            self.session_order.retain(|id| !active_ids.contains(id));
            self.session_order.extend(active_order);
            if let Err(error) = projects::save_app_session_order(&self.session_order) {
                self.sessions_error = Some(error);
            }
        }
        self.notify_session_rail(cx);
    }

    pub(super) fn complete_session_category_drop(
        &mut self,
        drag: &DraggedSession,
        target_kind: SessionRailKind,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.session_drop_target = None;
        let Some(path) = drag.path.clone() else {
            self.notify_session_rail(cx);
            return;
        };
        match target_kind {
            SessionRailKind::Project => self.set_session_active(path, cx),
            SessionRailKind::Archived => {
                self.notify_session_rail(cx);
                self.request_session_archive(path, true, window, cx);
            }
        }
    }

    fn set_session_project_filter(
        &mut self,
        project: Option<PathBuf>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.session_project_filter != project {
            self.session_project_filter = project;
            self.archived_sessions_expanded = false;
            self.notify_session_rail(cx);
        }
    }
}

#[cfg(test)]
#[path = "session_rail_tests.rs"]
mod tests;
