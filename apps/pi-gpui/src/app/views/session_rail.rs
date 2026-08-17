//! Session rail rendering and interaction.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use gpui::{
    Anchor, AnyElement, AppContext as _, CursorStyle, Empty, FontWeight, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Role, StatefulInteractiveElement as _,
    Styled as _, WeakEntity, div, list, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Icon, Sizable as _, Size,
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
};

use super::{
    super::PiApp,
    session_groups::{
        ActiveProjectItem, ProjectGroup, SessionRailItem, SessionRailKind, session_rail_groups,
    },
};
use crate::{
    assets::AppIcon,
    composer_sessions::session_target,
    primitives::{ButtonTone, FeedbackTone, button, feedback, icon_button},
    projects::DraftSession,
    sessions::root_session_for_path,
    theme::THEME,
};

impl PiApp {
    pub(super) fn render_sessions(&self, entity: WeakEntity<Self>) -> impl IntoElement {
        let new_entity = entity.clone();
        let add_project_entity = entity.clone();
        let search_focus = self.search_focus.clone();
        let new_session_project =
            selected_new_session_project(self.session_project_filter.as_deref(), &self.project);
        let selected_root =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .map(|session| session.id.clone());
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
        let archived_rows = archived_rail_rows(&grouped.archived, &self.collapsed_projects);
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
        let active_row_entity = entity.clone();
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
                    );
                    session_row(item, selected, badge, active_row_entity.clone())
                }
                None => div().into_any_element(),
            },
        )
        .size_full();

        let archived_live_status = self.snapshot.live_status.clone();
        let archived_run_statuses = self.run_statuses.clone();
        let archived_row_entity = entity.clone();
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
                        );
                        session_row(item, selected, badge, archived_row_entity.clone())
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
            .unwrap_or_else(|| "All projects".into());

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
                            .h(px(40.0))
                            .flex()
                            .items_center()
                            .gap(THEME.space.xs)
                            .child(
                                div()
                                    .id("session-search-surface")
                                    .h_full()
                                    .min_w_0()
                                    .flex_1()
                                    .flex()
                                    .items_center()
                                    .gap(THEME.space.sm)
                                    .px(THEME.space.sm)
                                    .rounded(THEME.radius)
                                    .bg(THEME.colors.surface)
                                    .text_color(THEME.colors.muted)
                                    .on_click(move |_, window, cx| {
                                        search_focus.focus(window, cx);
                                    })
                                    .child(
                                        Icon::new(AppIcon::MagnifyingGlass).with_size(Size::Small),
                                    )
                                    .child(
                                        Input::new(&self.search)
                                            .flex_1()
                                            .min_w_0()
                                            .appearance(false),
                                    ),
                            )
                            .child(icon_button(
                                "new-session",
                                AppIcon::ChatCircleDots,
                                "New session",
                                ButtonTone::Quiet,
                                true,
                                move |window, cx| {
                                    let _ = new_entity.update(cx, |this, cx| {
                                        this.new_session(new_session_project.clone(), window, cx);
                                    });
                                },
                            )),
                    )
                    .child(
                        div()
                            .h(px(40.0))
                            .flex()
                            .items_center()
                            .gap(THEME.space.xs)
                            .px(THEME.space.sm)
                            .rounded(THEME.radius)
                            .bg(THEME.colors.surface)
                            .text_color(THEME.colors.muted)
                            .child(Icon::new(AppIcon::Folder).with_size(Size::Small))
                            .child(
                                button(
                                    "project-filter",
                                    format!("{filter_label}  ▾"),
                                    ButtonTone::Quiet,
                                    true,
                                    |_, _| {},
                                )
                                .flex_1()
                                .min_w_0()
                                .dropdown_menu_with_anchor(
                                    Anchor::TopLeft,
                                    move |menu, _, _| {
                                        let all_entity = project_filter_entity.clone();
                                        let mut menu = menu
                                            .min_w(px(220.0))
                                            .max_h(px(420.0))
                                            .label("Projects")
                                            .item(PopupMenuItem::new("All projects").on_click(
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
                            )
                            .child(icon_button(
                                "add-project",
                                AppIcon::FolderPlus,
                                "Add project",
                                ButtonTone::Quiet,
                                true,
                                move |window, cx| {
                                    let _ = add_project_entity.update(cx, |this, cx| {
                                        this.choose_project_folder(window, cx);
                                    });
                                },
                            )),
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
                                        .h(px(40.0))
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
                                        .child(icon_button(
                                            "toggle-archived-sessions",
                                            if archived_expanded {
                                                AppIcon::Minus
                                            } else {
                                                AppIcon::Plus
                                            },
                                            if archived_expanded {
                                                "Collapse archived sessions"
                                            } else {
                                                "Expand archived sessions"
                                            },
                                            ButtonTone::Quiet,
                                            true,
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

#[derive(Clone)]
struct DraggedSession(String);

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

    fn toggle_project_group(&mut self, project: &Path, cx: &mut gpui::Context<Self>) {
        if !self.collapsed_projects.remove(project) {
            self.collapsed_projects.insert(project.to_path_buf());
        }
        self.notify_session_rail(cx);
    }
}

fn project_heading(project: &Path, collapsed: bool, entity: WeakEntity<PiApp>) -> AnyElement {
    let project_path = project.to_path_buf();
    let keyboard_project = project_path.clone();
    let keyboard_entity = entity.clone();
    div()
        .id(format!("project-group:{}", project.display()))
        .role(Role::Button)
        .aria_label(format!(
            "{} project {}",
            if collapsed { "Expand" } else { "Collapse" },
            project_label(project)
        ))
        .aria_expanded(!collapsed)
        .tab_index(0)
        .h(px(36.0))
        .w_full()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .px(THEME.space.md)
        .text_size(THEME.type_scale.body_small)
        .font_weight(FontWeight::MEDIUM)
        .text_color(THEME.colors.muted)
        .hover(|heading| heading.bg(THEME.colors.hover))
        .focus(|heading| {
            heading
                .border(THEME.border)
                .border_color(THEME.colors.accent)
        })
        .cursor_pointer()
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                window.prevent_default();
                let _ = keyboard_entity.update(cx, |this, cx| {
                    this.toggle_project_group(&keyboard_project, cx);
                });
            }
        })
        .on_click(move |_, _, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.toggle_project_group(&project_path, cx);
            });
        })
        .child(if collapsed { "▸" } else { "▾" })
        .child(Icon::new(AppIcon::Folder).with_size(Size::Small))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(project_label(project)),
        )
        .into_any_element()
}

fn draft_session_row(
    draft: &DraftSession,
    selected: bool,
    status: &str,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let id = draft.id.clone();
    let discard_id = id.clone();
    let project = draft.project.clone();
    let keyboard_id = id.clone();
    let keyboard_project = project.clone();
    let keyboard_entity = entity.clone();
    let discard_entity = entity.clone();
    let keyboard_discard_entity = discard_entity.clone();
    let keyboard_discard_id = discard_id.clone();
    div()
        .h(THEME.layout.session_row_height)
        .w_full()
        .px(THEME.space.sm)
        .child(
            div()
                .id(format!("session-{id}"))
                .role(Role::Button)
                .aria_label(format!("Open draft session in {}", project.display()))
                .aria_selected(selected)
                .tab_index(0)
                .size_full()
                .h(THEME.layout.session_row_height)
                .flex()
                .flex_col()
                .justify_center()
                .gap(THEME.space.xs)
                .px(THEME.space.sm)
                .rounded(THEME.radius)
                .bg(if selected {
                    THEME.colors.surface
                } else {
                    THEME.colors.panel
                })
                .hover(|row| row.bg(THEME.colors.hover))
                .focus(|row| row.border(THEME.border).border_color(THEME.colors.accent))
                .cursor(CursorStyle::PointingHand)
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        window.prevent_default();
                        let _ = keyboard_entity.update(cx, |this, cx| {
                            this.resume_draft(
                                keyboard_id.clone(),
                                keyboard_project.clone(),
                                window,
                                cx,
                            );
                        });
                    }
                })
                .on_click(move |_, window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.resume_draft(id.clone(), project.clone(), window, cx);
                    });
                })
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_size(THEME.type_scale.caption)
                        .text_color(if status == "Draft" {
                            THEME.colors.subtle
                        } else {
                            THEME.colors.accent
                        })
                        .child(status.to_owned())
                        .child(
                            div()
                                .id(format!("discard-{discard_id}"))
                                .role(Role::Button)
                                .aria_label("Discard draft")
                                .tab_index(0)
                                .p(THEME.space.xs)
                                .rounded(THEME.radius)
                                .hover(|button| button.bg(THEME.colors.hover))
                                .child(Icon::new(AppIcon::X).with_size(Size::Small))
                                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        window.prevent_default();
                                        let _ = keyboard_discard_entity.update(cx, |this, cx| {
                                            this.discard_draft(&keyboard_discard_id, window, cx);
                                        });
                                    }
                                })
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    let _ = discard_entity.update(cx, |this, cx| {
                                        this.discard_draft(&discard_id, window, cx);
                                    });
                                }),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(THEME.type_scale.body)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(THEME.colors.text)
                        .child("New session"),
                ),
        )
        .into_any_element()
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

fn is_meaningful_active_status(status: &str) -> bool {
    !matches!(status, "" | "Draft" | "Done" | "Ready" | "Idle")
}

fn session_badge(
    item: &SessionRailItem,
    explicit_status: Option<&str>,
    live_session_id: Option<&str>,
    live_status: &str,
) -> Option<String> {
    let status = explicit_status
        .and_then(normalized_session_status)
        .or_else(|| {
            (live_session_id == Some(item.session.id.as_str()))
                .then(|| normalized_session_status(live_status))
                .flatten()
        })
        .or_else(|| item.session.is_running.then(|| "Working".into()))
        .or_else(|| (item.kind == SessionRailKind::Project).then(|| "Done".into()));
    match (item.kind, status.as_deref()) {
        (SessionRailKind::Settled, Some("Done")) | (_, None) => None,
        _ => status,
    }
}

fn normalized_session_status(status: &str) -> Option<String> {
    match status {
        "" | "Idle" | "Ready" => None,
        status => Some(status.into()),
    }
}

fn session_row(
    item: &SessionRailItem,
    selected: bool,
    status: Option<String>,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let session = &item.session;
    let path = session.path.clone();
    let project = session.project.clone();
    let keyboard_path = path.clone();
    let keyboard_project = project.clone();
    let keyboard_entity = entity.clone();
    let drop_entity = entity.clone();
    let dragged_id = session.id.clone();
    let drop_target_id = session.id.clone();
    let settle_path = session.path.clone();
    let settle_entity = entity.clone();
    let keyboard_settle_path = settle_path.clone();
    let keyboard_settle_entity = settle_entity.clone();
    let age = relative_age(session.modified);
    let is_settled = item.kind == SessionRailKind::Settled;
    let status_color = match status.as_deref() {
        Some("Done") => THEME.colors.success,
        Some("Needs input") => THEME.colors.warning,
        Some(_) => THEME.colors.accent,
        None => THEME.colors.subtle,
    };
    let status_text = status.unwrap_or_default();
    let accessible_state = if status_text.is_empty() {
        "Archived"
    } else {
        status_text.as_str()
    };
    let accessible_label = session_accessible_label(&session.title, accessible_state, &age);
    let settle_label = if is_settled { "Restore" } else { "Settle" };
    let settle_icon = if is_settled {
        AppIcon::ArrowCounterClockwise
    } else {
        AppIcon::Archive
    };
    let row = div()
        .id(format!("session-{}", session.id))
        .role(Role::Button)
        .aria_label(accessible_label)
        .aria_selected(selected)
        .tab_index(0)
        .size_full()
        .h(THEME.layout.session_row_height)
        .flex()
        .flex_col()
        .justify_center()
        .gap(THEME.space.xs)
        .px(THEME.space.sm)
        .rounded(THEME.radius)
        .group(format!("session-actions-{}", session.id))
        .bg(if selected {
            THEME.colors.surface
        } else {
            THEME.colors.panel
        })
        .hover(|row| row.bg(THEME.colors.hover))
        .focus(|row| row.border(THEME.border).border_color(THEME.colors.accent))
        .cursor(CursorStyle::PointingHand)
        .on_drag(DraggedSession(dragged_id), |_, _, _, cx| cx.new(|_| Empty))
        .on_drop(move |dragged: &DraggedSession, _, cx| {
            let _ = drop_entity.update(cx, |this, cx| {
                this.move_session_to(&dragged.0, &drop_target_id, cx);
            });
        })
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                window.prevent_default();
                let _ = keyboard_entity.update(cx, |this, cx| {
                    this.resume(keyboard_path.clone(), keyboard_project.clone(), window, cx)
                });
            }
        })
        .on_click(move |_, window, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.resume(path.clone(), project.clone(), window, cx)
            });
        })
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .gap(THEME.space.sm)
                .text_size(THEME.type_scale.caption)
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_color(status_color)
                        .child(status_text),
                )
                .child(div().flex_none().text_color(THEME.colors.subtle).child(age)),
        )
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap(THEME.space.sm)
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(THEME.type_scale.body)
                        .font_weight(if selected || !is_settled {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if is_settled && !selected {
                            THEME.colors.muted
                        } else {
                            THEME.colors.text
                        })
                        .child(session.title.clone()),
                )
                .child(
                    div()
                        .id(format!("settle-{}", session.id))
                        .role(Role::Button)
                        .aria_label(format!("{settle_label} session"))
                        .tab_index(0)
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(THEME.space.xs)
                        .px(THEME.space.xs)
                        .py(px(2.0))
                        .border(THEME.border)
                        .border_color(THEME.colors.border)
                        .rounded(THEME.radius)
                        .opacity(0.0)
                        .group_hover(format!("session-actions-{}", session.id), |button| {
                            button.opacity(1.0)
                        })
                        .focus(|button| button.opacity(1.0))
                        .text_size(THEME.type_scale.caption)
                        .text_color(if is_settled {
                            THEME.colors.success
                        } else {
                            THEME.colors.muted
                        })
                        .hover(|button| button.bg(THEME.colors.hover))
                        .child(Icon::new(settle_icon).with_size(Size::Small))
                        .child(settle_label)
                        .on_key_down(move |event: &KeyDownEvent, window, cx| {
                            cx.stop_propagation();
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                window.prevent_default();
                                let _ = keyboard_settle_entity.update(cx, |this, cx| {
                                    this.set_session_settled(
                                        keyboard_settle_path.clone(),
                                        !is_settled,
                                        cx,
                                    );
                                });
                            }
                        })
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            let _ = settle_entity.update(cx, |this, cx| {
                                this.set_session_settled(settle_path.clone(), !is_settled, cx);
                            });
                        }),
                ),
        );
    div()
        .h(THEME.layout.session_row_height)
        .w_full()
        .px(THEME.space.sm)
        .child(row)
        .into_any_element()
}

fn selected_new_session_project(filter: Option<&Path>, current: &Path) -> PathBuf {
    filter.unwrap_or(current).to_path_buf()
}

fn session_accessible_label(title: &str, state: &str, age: &str) -> String {
    format!("Resume session: {title}. State: {state}. Updated {age}")
}

fn project_label(project: &Path) -> String {
    project
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| project.display().to_string(), str::to_owned)
}

fn relative_age(modified: SystemTime) -> String {
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO);
    if age < Duration::from_secs(60) {
        "now".into()
    } else if age < Duration::from_secs(60 * 60) {
        format!("{}m", age.as_secs() / 60)
    } else if age < Duration::from_secs(24 * 60 * 60) {
        format!("{}h", age.as_secs() / (60 * 60))
    } else {
        format!("{}d", age.as_secs() / (24 * 60 * 60))
    }
}

#[cfg(test)]
#[path = "session_rail_tests.rs"]
mod tests;
