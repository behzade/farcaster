use gpui::{
    Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use gpui_component::input::{Input, InputState, Textarea, TextareaState};

use super::{
    adapter::WorkGraphBoardView,
    contract::{BoardData, BoardFilter, IssueGroup, IssueRow},
    core::{filter_count, format_relative_issue_time},
    layout::{BoardLayoutMode, issue_detail_shell},
};
use crate::{
    primitives::{ButtonTone, button},
    theme::THEME,
};

pub(super) fn render_filter_rail(
    filter: BoardFilter,
    entity: Entity<WorkGraphBoardView>,
    data: &BoardData,
) -> impl IntoElement {
    div()
        .w(px(216.0))
        .min_w(px(216.0))
        .flex_none()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .px(THEME.space.sm)
        .py(THEME.space.md)
        .bg(THEME.colors.canvas)
        .border_r(THEME.border)
        .border_color(THEME.colors.border)
        .child(
            div()
                .px(THEME.space.sm)
                .pb(THEME.space.sm)
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child("WORK STATES"),
        )
        .children(BoardFilter::ALL.into_iter().map(|item| {
            let selected = item == filter;
            let count = filter_count(data, item);
            let entity = entity.clone();
            button(
                format!("workgraph-filter-{item:?}"),
                format!("{}  {count}", item.label()),
                if selected {
                    ButtonTone::Neutral
                } else {
                    ButtonTone::Quiet
                },
                true,
                move |_, cx| {
                    entity.update(cx, |this, cx| this.set_filter(item, cx));
                },
            )
        }))
}

pub(super) fn render_groups(
    selected: Option<u64>,
    active_session_id: Option<&str>,
    entity: Entity<WorkGraphBoardView>,
    groups: Vec<IssueGroup>,
    data: &BoardData,
) -> impl IntoElement {
    let current_issue = active_session_id.and_then(|session_id| {
        data.sessions
            .iter()
            .find(|link| link.session_id == session_id)
            .map(|link| link.issue_number)
    });
    div()
        .id("workgraph-issue-list")
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(THEME.space.md)
        .p(THEME.space.md)
        .children(
            groups
                .into_iter()
                .map(|group| render_group(group, selected, current_issue, entity.clone())),
        )
}

pub(super) fn render_graph(
    selected: Option<u64>,
    entity: Entity<WorkGraphBoardView>,
    data: &BoardData,
) -> impl IntoElement {
    let issues = data.issues.clone();
    let dependencies = data.dependencies.clone();
    div()
        .id("workgraph-dependency-list")
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .children(issues.into_iter().map(move |issue| {
            let dependency_titles = dependencies
                .iter()
                .filter(|edge| edge.issue_number == issue.number)
                .filter_map(|edge| {
                    data.issues
                        .iter()
                        .find(|candidate| candidate.number == edge.depends_on)
                        .map(|candidate| format!("#{} {}", candidate.number, candidate.title))
                })
                .collect::<Vec<_>>();
            render_graph_row(issue, dependency_titles, selected, entity.clone())
        }))
}

pub(super) fn render_create(
    title: &Entity<InputState>,
    body: &Entity<TextareaState>,
    entity: Entity<WorkGraphBoardView>,
    layout: BoardLayoutMode,
) -> impl IntoElement {
    let submit_title = title.clone();
    let submit_body = body.clone();
    let submit = entity.clone();
    let cancel = entity;
    let narrow = issue_detail_shell(layout).shows_sheet(false);
    div()
        .id("workgraph-create")
        .w(px(400.0))
        .min_w(px(360.0))
        .when(narrow, |form| form.w_full().min_w_0())
        .flex_none()
        .h_full()
        .overflow_y_scroll()
        .p(THEME.space.md)
        .bg(THEME.colors.panel)
        .border_l(THEME.border)
        .border_color(THEME.colors.border)
        .flex()
        .flex_col()
        .gap(THEME.space.md)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(THEME.space.xs)
                .child(
                    div()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child("NEW ISSUE"),
                )
                .child(
                    div()
                        .text_size(THEME.type_scale.display)
                        .text_color(THEME.colors.text)
                        .child("Record concrete project work"),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(THEME.space.xs)
                .child(
                    div()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child("TITLE"),
                )
                .child(Input::new(title).w_full()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(THEME.space.xs)
                .child(
                    div()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child("DESCRIPTION"),
                )
                .child(Textarea::new(body).w_full()),
        )
        .child(
            div()
                .flex()
                .gap(THEME.space.xs)
                .child(button(
                    "workgraph-create-submit",
                    "Create issue",
                    ButtonTone::Neutral,
                    true,
                    move |window, cx| {
                        let title = submit_title.read(cx).value().trim().to_owned();
                        if title.is_empty() {
                            return;
                        }
                        let body = submit_body.read(cx).value().trim().to_owned();
                        submit_title.update(cx, |input, cx| {
                            input.set_value(String::new(), window, cx);
                        });
                        submit_body.update(cx, |input, cx| {
                            input.set_value(String::new(), window, cx);
                        });
                        submit.update(cx, |this, cx| this.create_issue(title, body, cx));
                    },
                ))
                .child(button(
                    "workgraph-create-cancel",
                    "Cancel",
                    ButtonTone::Quiet,
                    true,
                    move |_, cx| {
                        cancel.update(cx, |this, cx| this.clear_selection(cx));
                    },
                )),
        )
}

pub(super) fn render_edit_fields(
    issue: workgraph::contract::Issue,
    title: &Entity<InputState>,
    body: &Entity<TextareaState>,
    priority: &Entity<InputState>,
    entity: Entity<WorkGraphBoardView>,
) -> impl IntoElement {
    let submit_title = title.clone();
    let submit_body = body.clone();
    let submit_priority = priority.clone();
    let submit = entity.clone();
    let cancel = entity;
    let number = issue.number;
    let version = issue.version;
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.md)
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child(format!("EDIT ISSUE #{number}")),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(THEME.space.xs)
                .child(
                    div()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child("TITLE"),
                )
                .child(Input::new(title).w_full()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(THEME.space.xs)
                .child(
                    div()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child("DESCRIPTION"),
                )
                .child(Textarea::new(body).w_full()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(THEME.space.xs)
                .child(
                    div()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child("PRIORITY"),
                )
                .child(Input::new(priority).w(px(120.0))),
        )
        .child(
            div()
                .flex()
                .gap(THEME.space.xs)
                .child(button(
                    format!("workgraph-edit-save-{number}"),
                    "Save changes",
                    ButtonTone::Neutral,
                    true,
                    move |_, cx| {
                        let title = submit_title.read(cx).value().trim().to_owned();
                        let body = submit_body.read(cx).value().to_string();
                        let Ok(priority) = submit_priority.read(cx).value().trim().parse::<u64>()
                        else {
                            return;
                        };
                        if title.is_empty() {
                            return;
                        }
                        submit.update(cx, |this, cx| {
                            this.update_issue_fields(number, title, body, priority, version, cx);
                        });
                    },
                ))
                .child(button(
                    format!("workgraph-edit-cancel-{number}"),
                    "Cancel",
                    ButtonTone::Quiet,
                    true,
                    move |_, cx| {
                        cancel.update(cx, |this, cx| this.set_editing(None, cx));
                    },
                )),
        )
}

pub(super) fn related_issue_section(
    label: &'static str,
    empty: &'static str,
    issues: Vec<workgraph::contract::Issue>,
    entity: Entity<WorkGraphBoardView>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child(label),
        )
        .when(issues.is_empty(), |section| {
            section.child(
                div()
                    .text_size(THEME.type_scale.body_small)
                    .text_color(THEME.colors.muted)
                    .child(empty),
            )
        })
        .children(issues.into_iter().map(|issue| {
            let number = issue.number;
            let entity = entity.clone();
            div()
                .id(format!("workgraph-related-{label}-{number}"))
                .cursor_pointer()
                .rounded(THEME.radius)
                .px(THEME.space.sm)
                .py(THEME.space.xs)
                .bg(THEME.colors.surface)
                .hover(|style| style.bg(THEME.colors.hover))
                .text_size(THEME.type_scale.body_small)
                .text_color(THEME.colors.link)
                .child(format!("#{number}  {}", issue.title))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.select_issue(number, cx));
                })
        }))
}

pub(super) fn dependency_issue_section(
    issue_number: u64,
    issue_version: u64,
    dependencies: Vec<workgraph::contract::Issue>,
    entity: Entity<WorkGraphBoardView>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child("DEPENDS ON"),
        )
        .when(dependencies.is_empty(), |section| {
            section.child(
                div()
                    .text_size(THEME.type_scale.body_small)
                    .text_color(THEME.colors.muted)
                    .child("Nothing — this issue can move independently."),
            )
        })
        .children(dependencies.into_iter().map(|dependency| {
            let depends_on = dependency.number;
            let open = entity.clone();
            let remove = entity.clone();
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(THEME.space.xs)
                .child(
                    div()
                        .id(format!("workgraph-dependency-{issue_number}-{depends_on}"))
                        .flex_1()
                        .min_w_0()
                        .cursor_pointer()
                        .rounded(THEME.radius)
                        .px(THEME.space.sm)
                        .py(THEME.space.xs)
                        .bg(THEME.colors.surface)
                        .hover(|style| style.bg(THEME.colors.hover))
                        .text_size(THEME.type_scale.body_small)
                        .text_color(THEME.colors.link)
                        .child(format!("#{depends_on}  {}", dependency.title))
                        .on_click(move |_, _, cx| {
                            open.update(cx, |this, cx| this.select_issue(depends_on, cx));
                        }),
                )
                .child(button(
                    format!("workgraph-remove-dependency-{issue_number}-{depends_on}"),
                    "Remove",
                    ButtonTone::Quiet,
                    true,
                    move |_, cx| {
                        remove.update(cx, |this, cx| {
                            this.change_dependency(
                                issue_number,
                                depends_on,
                                issue_version,
                                false,
                                cx,
                            );
                        });
                    },
                ))
        }))
}

pub(super) fn detail_section(label: &'static str, body: String) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child(label),
        )
        .child(
            div()
                .text_size(THEME.type_scale.body_small)
                .text_color(THEME.colors.muted)
                .line_height(THEME.type_scale.line_body)
                .child(body),
        )
}

pub(super) fn status_color(status: workgraph::contract::IssueStatus) -> gpui::Rgba {
    match status {
        workgraph::contract::IssueStatus::Blocked => THEME.colors.warning,
        workgraph::contract::IssueStatus::Done => THEME.colors.success,
        workgraph::contract::IssueStatus::Cancelled => THEME.colors.subtle,
        workgraph::contract::IssueStatus::InProgress => THEME.colors.accent,
        workgraph::contract::IssueStatus::Open => THEME.colors.link,
    }
}

pub(super) fn render_group(
    group: IssueGroup,
    selected: Option<u64>,
    current_issue: Option<u64>,
    entity: Entity<WorkGraphBoardView>,
) -> impl IntoElement {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default();
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child(format!("{}  {}", group.group.label(), group.rows.len())),
        )
        .children(
            group
                .rows
                .into_iter()
                .map(|row| render_issue_row(row, selected, current_issue, now, entity.clone())),
        )
}

fn render_issue_row(
    row: IssueRow,
    selected: Option<u64>,
    current_issue: Option<u64>,
    now: i64,
    entity: Entity<WorkGraphBoardView>,
) -> impl IntoElement {
    let row_status_color = if row.status_label.starts_with("Blocked") {
        THEME.colors.warning
    } else {
        status_color(row.issue.status)
    };
    let number = row.issue.number;
    let is_selected = selected == Some(number);
    div()
        .id(format!("workgraph-issue-{number}"))
        .cursor_pointer()
        .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.select_issue(number, cx)))
        .rounded(THEME.radius)
        .border(THEME.border)
        .border_color(if is_selected {
            THEME.colors.accent
        } else {
            THEME.colors.border
        })
        .bg(if is_selected {
            THEME.colors.selection
        } else {
            THEME.colors.surface
        })
        .hover(|style| style.bg(THEME.colors.hover))
        .px(THEME.space.sm)
        .py(THEME.space.sm)
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(THEME.space.sm)
                .child(
                    div()
                        .min_w_0()
                        .text_color(THEME.colors.text)
                        .child(format!("#{}  {}", row.issue.number, row.issue.title)),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(THEME.space.xs)
                        .when(current_issue == Some(number), |meta| {
                            meta.child(
                                div()
                                    .rounded(THEME.radius)
                                    .px(THEME.space.xs)
                                    .bg(THEME.colors.accent_active)
                                    .text_size(THEME.type_scale.caption)
                                    .text_color(THEME.colors.canvas)
                                    .child("Current session"),
                            )
                        })
                        .child(
                            div()
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.muted)
                                .child(format!(
                                    "{}  ·  {}",
                                    row.priority_label,
                                    format_relative_issue_time(row.issue.updated_at, now)
                                )),
                        ),
                ),
        )
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(row_status_color)
                .child(row.status_label),
        )
        .when(!row.issue.body.trim().is_empty(), |item| {
            item.child(
                div()
                    .text_size(THEME.type_scale.body_small)
                    .text_color(THEME.colors.muted)
                    .child(row.issue.body.lines().next().unwrap_or_default().to_owned()),
            )
        })
}

pub(super) fn render_graph_row(
    issue: workgraph::contract::Issue,
    dependencies: Vec<String>,
    selected: Option<u64>,
    entity: Entity<WorkGraphBoardView>,
) -> impl IntoElement {
    let number = issue.number;
    let is_selected = selected == Some(number);
    div()
        .id(format!("workgraph-graph-{number}"))
        .cursor_pointer()
        .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.select_issue(number, cx)))
        .px(THEME.space.md)
        .py(THEME.space.sm)
        .border_b(THEME.border)
        .border_color(THEME.colors.border)
        .bg(if is_selected {
            THEME.colors.selection
        } else {
            THEME.colors.panel
        })
        .hover(|style| style.bg(THEME.colors.hover))
        .flex()
        .items_center()
        .gap(THEME.space.md)
        .child(
            div()
                .w(px(220.0))
                .text_color(THEME.colors.text)
                .child(format!("#{number}  {}", issue.title)),
        )
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child(if dependencies.is_empty() {
                    "Ready root".into()
                } else {
                    format!("← {}", dependencies.join(", "))
                }),
        )
}
