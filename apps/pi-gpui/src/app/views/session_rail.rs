//! Session rail rendering and interaction.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use gpui::{
    Anchor, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, list,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
};

use super::{
    super::PiApp,
    session_groups::{
        ActiveProjectItem, ProjectGroup, SessionRailItem, recent_archived_sessions,
        session_rail_groups,
    },
    session_rows::{
        draft_session_row, project_heading, project_label, session_badge, session_row,
        session_row_with_height,
    },
};
#[cfg(test)]
use super::{session_groups::SessionRailKind, session_rows::session_accessible_label};
use crate::{
    assets::AppIcon,
    composer_sessions::session_target,
    primitives::{
        AppIconSize, ButtonTone, FeedbackTone, app_icon, disclosure_button, dropdown_button,
        dropdown_icon_button, feedback, icon_button,
    },
    projects::DraftSession,
    sessions::{SessionSummary, root_session_for_path},
    theme::THEME,
};

const ARCHIVED_PREVIEW_LIMIT: usize = 3;

impl PiApp {
    pub(super) fn render_sessions(&self, entity: WeakEntity<Self>) -> impl IntoElement {
        let new_entity = entity.clone();
        let menu_new_entity = entity.clone();
        let add_project_entity = entity.clone();
        let menu_add_project_entity = entity.clone();
        let menu_workgraph_entity = entity.clone();
        let search_focus = self.search_focus.clone();
        let new_session_project =
            selected_new_session_project(self.session_project_filter.as_deref(), &self.project);
        let menu_new_session_project = new_session_project.clone();
        let selected_root =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .map(|session| session.id.clone());
        let live_root =
            root_session_for_path(&self.sessions, self.snapshot.live_session.as_deref())
                .map(|session| session.id.clone());
        let live_project = (!self.snapshot.project.as_os_str().is_empty())
            .then_some(self.snapshot.project.as_path());
        let waiting_roots = roots_waiting_for_descendants(&self.all_sessions);
        let active_projects = resolved_active_projects(
            &self.sessions,
            &self.drafts,
            &self.submitted_drafts,
            &self.run_statuses,
            live_root.as_deref(),
            &self.snapshot.live_status,
            live_project,
        );
        let grouped = session_rail_groups(
            &self.sessions,
            &self.drafts,
            &self.session_order,
            self.session_project_filter.as_deref(),
            &active_projects,
        );
        let active_entry_count = grouped
            .active
            .iter()
            .map(|group| group.items.len())
            .sum::<usize>();
        let archived_entry_count = grouped
            .archived
            .iter()
            .map(|group| group.items.len())
            .sum::<usize>();
        let active_rows = active_rail_rows(&grouped.active, &self.collapsed_projects);
        let session_shortcuts = visible_session_shortcuts(&active_rows);
        let archived_rows = archived_rail_rows(&grouped.archived, &self.collapsed_projects);
        let archived_preview = recent_archived_sessions(&grouped.archived, ARCHIVED_PREVIEW_LIMIT);
        reconcile_list_rows(
            &self.session_list,
            &self.session_list_rows,
            active_rows.iter().map(ActiveRailRow::identity).collect(),
        );
        reconcile_list_rows(
            &self.archived_session_list,
            &self.archived_session_list_rows,
            archived_rows
                .iter()
                .map(ArchivedRailRow::identity)
                .collect(),
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
        let shortcuts_visible = self.session_shortcuts_visible;
        let active_list = list(
            self.session_list.clone(),
            move |index, _, _| match active_rows.get(index) {
                Some(ActiveRailRow::Project(project, collapsed)) => {
                    project_heading(project, *collapsed, active_row_entity.clone())
                }
                Some(ActiveRailRow::Draft(draft)) => {
                    let selected = selected_draft.as_deref() == Some(draft.id.as_str());
                    let status = crate::app::drafts::resolved_draft_status(
                        &draft.id,
                        &submitted_drafts,
                        &active_run_statuses,
                    );
                    draft_session_row(draft, selected, &status, active_row_entity.clone())
                }
                Some(ActiveRailRow::Session(item)) => {
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
                        .then(|| session_shortcuts.get(&item.session.id).copied())
                        .flatten();
                    let editing =
                        active_editing_path.as_deref() == Some(item.session.path.as_path());
                    session_row(
                        item,
                        selected,
                        badge,
                        shortcut,
                        editing.then(|| active_title_input.clone()),
                        active_row_entity.clone(),
                    )
                }
                None => div().into_any_element(),
            },
        )
        .size_full();

        let archived_preview_elements = archived_preview
            .iter()
            .map(|item| {
                let selected = selected_root.as_deref() == Some(item.session.id.as_str());
                let target = format!("session:{}", item.session.path.display());
                let badge = session_badge(
                    item,
                    self.run_statuses.get(&target).map(String::as_str),
                    live_root.as_deref(),
                    &self.snapshot.live_status,
                    waiting_roots.contains(&item.session.id),
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
                    editing.then(|| self.session_title_input.clone()),
                    THEME.controls.archived_preview_row,
                    entity.clone(),
                )
            })
            .collect::<Vec<_>>();
        let archived_live_status = self.snapshot.live_status.clone();
        let archived_run_statuses = self.run_statuses.clone();
        let archived_waiting_roots = waiting_roots;
        let archived_row_entity = entity.clone();
        let archived_editing_path = self
            .editing_session_title
            .as_ref()
            .map(|edit| edit.path.clone());
        let archived_title_input = self.session_title_input.clone();
        let archived_list =
            list(
                self.archived_session_list.clone(),
                move |index, _, _| match archived_rows.get(index) {
                    Some(ArchivedRailRow::Project(project, collapsed)) => {
                        project_heading(project, *collapsed, archived_row_entity.clone())
                    }
                    Some(ArchivedRailRow::Session(item)) => {
                        let selected = selected_root.as_deref() == Some(item.session.id.as_str());
                        let target = format!("session:{}", item.session.path.display());
                        let badge = session_badge(
                            item,
                            archived_run_statuses.get(&target).map(String::as_str),
                            live_root.as_deref(),
                            &archived_live_status,
                            archived_waiting_roots.contains(&item.session.id),
                        );
                        let editing =
                            archived_editing_path.as_deref() == Some(item.session.path.as_path());
                        session_row(
                            item,
                            selected,
                            badge,
                            None,
                            editing.then(|| archived_title_input.clone()),
                            archived_row_entity.clone(),
                        )
                    }
                    None => div().into_any_element(),
                },
            )
            .size_full();

        let archived_expanded = self.archived_sessions_expanded && archived_entry_count > 0;
        let archive_toggle_entity = entity.clone();
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
                            .child(
                                dropdown_icon_button(
                                    "session-menu",
                                    AppIcon::List,
                                    "Menu",
                                    ButtonTone::Quiet,
                                    true,
                                )
                                .dropdown_menu_with_anchor(
                                    Anchor::TopLeft,
                                    move |menu, _, _| {
                                        let new_entity = menu_new_entity.clone();
                                        let add_project_entity = menu_add_project_entity.clone();
                                        let project = menu_new_session_project.clone();
                                        let workgraph_entity = menu_workgraph_entity.clone();
                                        menu.label("Menu")
                                            .item(PopupMenuItem::new("New session").on_click(
                                                move |_, window, cx| {
                                                    let _ = new_entity.update(cx, |this, cx| {
                                                        this.new_session(
                                                            project.clone(),
                                                            window,
                                                            cx,
                                                        );
                                                    });
                                                },
                                            ))
                                            .item(PopupMenuItem::new("New project").on_click(
                                                move |_, window, cx| {
                                                    let _ = add_project_entity.update(
                                                        cx,
                                                        |this, cx| {
                                                            this.choose_project_folder(window, cx);
                                                        },
                                                    );
                                                },
                                            ))
                                            .separator()
                                            .item(PopupMenuItem::new("Work graph").on_click(
                                                move |_, window, cx| {
                                                    let _ =
                                                        workgraph_entity.update(cx, |this, cx| {
                                                            this.open_workgraph_surface(window, cx);
                                                        });
                                                },
                                            ))
                                    },
                                ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(THEME.space.xs)
                                    .child(icon_button(
                                        "new-session",
                                        AppIcon::Plus,
                                        "New session",
                                        ButtonTone::Quiet,
                                        move |window, cx| {
                                            let _ = new_entity.update(cx, |this, cx| {
                                                this.new_session(
                                                    new_session_project.clone(),
                                                    window,
                                                    cx,
                                                );
                                            });
                                        },
                                    ))
                                    .child(icon_button(
                                        "add-project",
                                        AppIcon::FolderPlus,
                                        "New project",
                                        ButtonTone::Quiet,
                                        move |window, cx| {
                                            let _ = add_project_entity.update(cx, |this, cx| {
                                                this.choose_project_folder(window, cx);
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
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_y_hidden()
                    .when(!archived_expanded, |lists| {
                        lists.child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .overflow_y_hidden()
                                .child(active_list),
                        )
                    })
                    .when(archived_entry_count > 0, |lists| {
                        lists.child(
                            div()
                                .id("archived-sessions")
                                .min_h_0()
                                .flex()
                                .flex_col()
                                .when(archived_expanded, |archived| archived.flex_1())
                                .when(!archived_expanded, |archived| archived.flex_none())
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
                                        .child(format!("Archived · {archived_entry_count}"))
                                        .child(disclosure_button(
                                            "toggle-archived-sessions",
                                            archived_expanded,
                                            "Archived sessions",
                                            move |_, cx| {
                                                let _ =
                                                    archive_toggle_entity.update(cx, |this, cx| {
                                                        this.archived_sessions_expanded =
                                                            !this.archived_sessions_expanded;
                                                        this.notify_session_rail(cx);
                                                    });
                                            },
                                        )),
                                )
                                .when(!archived_expanded, |archived| {
                                    archived.children(archived_preview_elements)
                                })
                                .when(archived_expanded, |archived| {
                                    archived.child(
                                        div()
                                            .flex_1()
                                            .min_h_0()
                                            .overflow_y_hidden()
                                            .child(archived_list),
                                    )
                                }),
                        )
                    }),
            )
            .when(
                active_entry_count == 0
                    && (!archived_expanded || archived_entry_count == 0)
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
}

#[derive(Clone, Debug)]
enum ActiveRailRow {
    Project(PathBuf, bool),
    Draft(DraftSession),
    Session(SessionRailItem),
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
enum ArchivedRailRow {
    Project(PathBuf, bool),
    Session(SessionRailItem),
}

impl ActiveRailRow {
    fn identity(&self) -> String {
        match self {
            Self::Project(project, _) => format!("project:{project:?}"),
            Self::Draft(draft) => format!("draft:{}", draft.id),
            Self::Session(item) => format!("session:{}", item.session.id),
        }
    }
}

impl ArchivedRailRow {
    fn identity(&self) -> String {
        match self {
            Self::Project(project, _) => format!("project:{project:?}"),
            Self::Session(item) => format!("session:{}", item.session.id),
        }
    }
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

fn active_rail_rows(
    groups: &[ProjectGroup<ActiveProjectItem>],
    collapsed_projects: &HashSet<PathBuf>,
) -> Vec<ActiveRailRow> {
    let mut rows = Vec::new();
    for group in groups {
        let collapsed = collapsed_projects.contains(&group.project);
        rows.push(ActiveRailRow::Project(group.project.clone(), collapsed));
        if !collapsed {
            rows.extend(group.items.iter().cloned().map(|item| match item {
                ActiveProjectItem::Draft(draft) => ActiveRailRow::Draft(draft),
                ActiveProjectItem::Session(session) => ActiveRailRow::Session(session),
            }));
        }
    }
    rows
}

fn visible_session_shortcuts(rows: &[ActiveRailRow]) -> HashMap<String, u8> {
    rows.iter()
        .filter_map(|row| match row {
            ActiveRailRow::Session(item) => Some(item.session.id.clone()),
            ActiveRailRow::Project(_, _) | ActiveRailRow::Draft(_) => None,
        })
        .take(9)
        .enumerate()
        .map(|(index, id)| (id, (index + 1) as u8))
        .collect()
}

fn archived_rail_rows(
    groups: &[ProjectGroup<SessionRailItem>],
    collapsed_projects: &HashSet<PathBuf>,
) -> Vec<ArchivedRailRow> {
    let mut rows = Vec::new();
    for group in groups {
        let collapsed = collapsed_projects.contains(&group.project);
        rows.push(ArchivedRailRow::Project(group.project.clone(), collapsed));
        if !collapsed {
            rows.extend(group.items.iter().cloned().map(ArchivedRailRow::Session));
        }
    }
    rows
}

impl PiApp {
    pub(super) fn switch_to_session_number(
        &mut self,
        number: usize,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(session) = self
            .visible_sessions()
            .get(number.saturating_sub(1))
            .cloned()
        {
            self.select_session(session.path, session.project, window, cx);
        }
    }

    pub(super) fn switch_relative_session(
        &mut self,
        direction: isize,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let sessions = self.visible_sessions();
        let selected =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .and_then(|selected| {
                    sessions
                        .iter()
                        .position(|session| session.id == selected.id)
                });
        let Some(current) = selected else { return };
        let next = current as isize + direction;
        if next >= 0
            && let Some(session) = sessions.get(next as usize).cloned()
        {
            self.select_session(session.path, session.project, window, cx);
        }
    }

    fn visible_sessions(&self) -> Vec<crate::sessions::SessionSummary> {
        let live_root =
            root_session_for_path(&self.sessions, self.snapshot.live_session.as_deref())
                .map(|session| session.id.clone());
        let live_project = (!self.snapshot.project.as_os_str().is_empty())
            .then_some(self.snapshot.project.as_path());
        let active_projects = resolved_active_projects(
            &self.sessions,
            &self.drafts,
            &self.submitted_drafts,
            &self.run_statuses,
            live_root.as_deref(),
            &self.snapshot.live_status,
            live_project,
        );
        let grouped = session_rail_groups(
            &self.sessions,
            &self.drafts,
            &self.session_order,
            self.session_project_filter.as_deref(),
            &active_projects,
        );
        active_rail_rows(&grouped.active, &self.collapsed_projects)
            .into_iter()
            .filter_map(|row| match row {
                ActiveRailRow::Session(item) => Some(item.session),
                ActiveRailRow::Project(_, _) | ActiveRailRow::Draft(_) => None,
            })
            .collect()
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

    pub(super) fn toggle_project_group(&mut self, project: &Path, cx: &mut gpui::Context<Self>) {
        if !self.collapsed_projects.remove(project) {
            self.collapsed_projects.insert(project.to_path_buf());
        }
        self.notify_session_rail(cx);
    }
}

fn selected_new_session_project(filter: Option<&Path>, current: &Path) -> PathBuf {
    filter.unwrap_or(current).to_path_buf()
}

fn resolved_active_projects(
    sessions: &[crate::sessions::SessionSummary],
    drafts: &[DraftSession],
    submitted_drafts: &HashMap<String, Option<PathBuf>>,
    run_statuses: &HashMap<String, String>,
    live_root: Option<&str>,
    live_status: &str,
    live_project: Option<&Path>,
) -> HashSet<PathBuf> {
    let mut active = sessions
        .iter()
        .filter(|session| session.is_running)
        .map(|session| session.project.clone())
        .collect::<HashSet<_>>();

    for session in sessions {
        if run_statuses
            .get(&session_target(&session.path))
            .is_some_and(|status| is_meaningful_active_status(status))
        {
            active.insert(session.project.clone());
        }
    }
    for draft in drafts {
        let status =
            crate::app::drafts::resolved_draft_status(&draft.id, submitted_drafts, run_statuses);
        if is_meaningful_active_status(&status) {
            active.insert(draft.project.clone());
        }
    }
    if live_root.is_some() && is_meaningful_active_status(live_status) {
        let project = live_root
            .and_then(|id| sessions.iter().find(|session| session.id == id))
            .map(|session| session.project.as_path())
            .or(live_project);
        if let Some(project) = project {
            active.insert(project.to_path_buf());
        }
    }

    active
}

fn roots_waiting_for_descendants(sessions: &[SessionSummary]) -> HashSet<String> {
    let parent_by_id = sessions
        .iter()
        .filter_map(|session| {
            session
                .parent_session
                .as_ref()
                .map(|parent| (session.id.as_str(), parent.as_str()))
        })
        .collect::<HashMap<_, _>>();
    let mut waiting = HashSet::new();
    for session in sessions.iter().filter(|session| session.is_running) {
        let mut current = session.id.as_str();
        let mut seen = HashSet::new();
        while seen.insert(current) {
            let Some(parent) = parent_by_id.get(current).copied() else {
                break;
            };
            waiting.insert(parent.to_owned());
            current = parent;
        }
    }
    waiting
}

fn is_meaningful_active_status(status: &str) -> bool {
    !matches!(status, "" | "Draft" | "Done" | "Ready" | "Idle")
}

#[cfg(test)]
#[path = "session_rail_tests.rs"]
mod tests;
