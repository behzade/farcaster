use gpui::{
    Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};

use super::{
    adapter::WorkGraphBoardView,
    contract::{IssueGroup, IssueRow},
    core::format_relative_issue_time,
};
use crate::{
    primitives::{ButtonTone, button},
    theme::THEME,
};

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
