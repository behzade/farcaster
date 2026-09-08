use gpui::{
    Div, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use gpui_component::input::{Input, InputState, Textarea, TextareaState};

use super::{
    adapter::{CreateStage, WorkGraphBoardView},
    contract::PlanRow,
};
use crate::{
    app::ui::assets::AppIcon,
    app::ui::primitives::{AppIconSize, ButtonTone, app_icon, button},
    app::ui::theme::THEME,
};

pub(super) fn render_session_goal(goal: &crate::agents::SessionGoal, compact: bool) -> Div {
    let status = goal.status.replace('_', " ");
    let usage = goal_usage_label(goal);
    div()
        .mx(if compact { px(0.0) } else { THEME.space.md })
        .mt(if compact { px(0.0) } else { THEME.space.sm })
        .px(THEME.space.sm)
        .py(THEME.space.sm)
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.surface)
        .flex()
        .items_start()
        .gap(THEME.space.sm)
        .child(
            div()
                .h(px(20.0))
                .flex()
                .items_center()
                .text_color(THEME.colors.accent)
                .child(app_icon(AppIcon::Eye, AppIconSize::Inline)),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(THEME.space.xs)
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child("Native goal")
                        .child("·")
                        .child(status),
                )
                .child(
                    div()
                        .line_clamp(if compact { 3 } else { 2 })
                        .text_size(THEME.type_scale.body_small)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(THEME.colors.text)
                        .child(goal.objective.clone()),
                )
                .when_some(usage, |content, usage| {
                    content.child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(usage),
                    )
                }),
        )
}

fn goal_usage_label(goal: &crate::agents::SessionGoal) -> Option<String> {
    let tokens = goal
        .token_budget
        .map(|budget| format!("{} / {budget} tokens", goal.tokens_used));
    let time = (goal.time_used_seconds > 0).then(|| {
        let minutes = goal.time_used_seconds / 60;
        if minutes > 0 {
            format!("{minutes}m elapsed")
        } else {
            format!("{}s elapsed", goal.time_used_seconds)
        }
    });
    match (tokens, time) {
        (Some(tokens), Some(time)) => Some(format!("{tokens} · {time}")),
        (Some(tokens), None) => Some(tokens),
        (None, Some(time)) => Some(time),
        (None, None) => None,
    }
}

pub(super) fn render_plan_list(
    rows: Vec<PlanRow>,
    selected: Option<u64>,
    entity: Entity<WorkGraphBoardView>,
) -> impl IntoElement {
    div()
        .id("workgraph-plan-list")
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .p(THEME.space.md)
        .when(rows.is_empty(), |list| {
            list.child(detail_empty("No nodes match your search."))
        })
        .children(
            rows.into_iter()
                .map(|row| render_plan_row(row, selected, entity.clone())),
        )
}

fn render_plan_row(
    row: PlanRow,
    selected: Option<u64>,
    entity: Entity<WorkGraphBoardView>,
) -> impl IntoElement {
    let number = row.node.number;
    let is_selected = selected == Some(number);
    let title_color = if row.detached || row.reached {
        THEME.colors.subtle
    } else {
        THEME.colors.text
    };
    div()
        .id(format!("workgraph-node-{number}"))
        .role(Role::Button)
        .aria_label(format!("Open plan node {}", row.node.title))
        .tab_index(0)
        .on_mouse_down(
            gpui::MouseButton::Left,
            crate::app::ui::primitives::preserve_pointer_focus,
        )
        .cursor_pointer()
        .flex_none()
        .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.select_node(number, cx)))
        .border_l(px(2.0))
        .border_color(if is_selected {
            THEME.colors.accent
        } else {
            THEME.colors.panel
        })
        .bg(if is_selected {
            THEME.colors.selection
        } else {
            THEME.colors.panel
        })
        .hover(|style| style.bg(THEME.colors.hover))
        .px(THEME.space.sm)
        .py(THEME.space.sm)
        .flex()
        .items_start()
        .gap(THEME.space.sm)
        .child(
            div()
                .w(px(22.0))
                .h(px(22.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .text_color(if row.reached {
                    THEME.colors.success
                } else if row.current {
                    THEME.colors.accent
                } else {
                    THEME.colors.subtle
                })
                .when(row.reached, |marker| {
                    marker.child(app_icon(AppIcon::CheckCircle, AppIconSize::Inline))
                })
                .when(!row.reached, |marker| {
                    marker.child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{number}")),
                    )
                }),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(
                    div()
                        .text_size(THEME.type_scale.body)
                        .font_weight(if row.current {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(title_color)
                        .when(row.reached, |title| title.line_through())
                        .child(row.node.title),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(THEME.space.xs)
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .when(row.current, |meta| meta.child("Current"))
                        .when(row.detached, |meta| meta.child("Detached"))
                        .when(!row.node.files.is_empty(), |meta| {
                            meta.child(format!("{} path(s)", row.node.files.len()))
                        }),
                ),
        )
}

pub(super) fn render_create_form(
    title: &Entity<InputState>,
    detail: &Entity<TextareaState>,
    stage: CreateStage,
    can_submit: bool,
    entity: Entity<WorkGraphBoardView>,
) -> impl IntoElement {
    let add_node = stage == CreateStage::Node;
    let cancel = entity.clone();
    let submit = entity;

    div()
        .size_full()
        .min_h_0()
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(56.0))
                .flex_none()
                .pl(px(24.0))
                .pr(px(56.0))
                .flex()
                .items_center()
                .justify_between()
                .border_b(THEME.border)
                .border_color(THEME.colors.surface)
                .child(
                    div()
                        .text_size(THEME.type_scale.display)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if add_node { "Add node" } else { "New plan" }),
                ),
        )
        .child(
            div()
                .id("workgraph-create-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p(px(24.0))
                .flex()
                .justify_center()
                .child(
                    div()
                        .w_full()
                        .max_w(px(520.0))
                        .when(add_node, |form| {
                            form.child(compact_field("Node title", Input::new(title).w_full()))
                                .child(div().mt(THEME.space.md).child(compact_field(
                                    "Paths (optional, one per line)",
                                    Textarea::new(detail).w_full().appearance(true),
                                )))
                        })
                        .when(!add_node, |form| {
                            form.child(
                                div()
                                    .mb(THEME.space.md)
                                    .text_size(THEME.type_scale.body_small)
                                    .text_color(THEME.colors.muted)
                                    .child("Describe where the project is now and what should be true when this plan is complete."),
                            )
                            .child(compact_field(
                                "Current state",
                                Textarea::new(detail).w_full().appearance(true),
                            ))
                            .child(div().mt(THEME.space.md).child(compact_field(
                                "Desired outcome",
                                Input::new(title).w_full(),
                            )))
                        }),
                ),
        )
        .child(
            div()
                .h(px(56.0))
                .flex_none()
                .px(px(24.0))
                .flex()
                .items_center()
                .justify_end()
                .gap(THEME.space.sm)
                .border_t(THEME.border)
                .border_color(THEME.colors.surface)
                .child(button(
                        "workgraph-create-cancel",
                        "Cancel",
                        ButtonTone::Quiet,
                        true,
                        move |window, cx| {
                            cancel.update(cx, |this, cx| {
                                this.cancel_create(window, cx);
                            });
                        },
                    ))
                .child(button(
                        "workgraph-create-submit",
                        if add_node { "Add node" } else { "Create plan" },
                        ButtonTone::Accent,
                        can_submit,
                        move |window, cx| {
                            submit.update(cx, |this, cx| {
                                this.submit_create_inputs(window, cx);
                            });
                        },
                    )),
        )
}

fn compact_field(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .text_size(THEME.type_scale.body_small)
                .font_weight(FontWeight::SEMIBOLD)
                .child(label),
        )
        .child(control)
}

pub(super) fn detail_section(label: &'static str) -> Div {
    div().flex().flex_col().gap(THEME.space.xs).child(
        div()
            .text_size(THEME.type_scale.caption)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(THEME.colors.muted)
            .child(label),
    )
}

pub(super) fn detail_rule() -> Div {
    div().h(THEME.border).w_full().bg(THEME.colors.surface)
}

pub(super) fn detail_empty(text: &'static str) -> Div {
    div()
        .text_size(THEME.type_scale.body_small)
        .text_color(THEME.colors.subtle)
        .child(text)
}

pub(super) fn detail_action(control: impl IntoElement) -> Div {
    div().flex().child(control)
}

pub(super) fn detail_copy() -> Div {
    div()
        .text_size(THEME.type_scale.body_small)
        .line_height(THEME.type_scale.line_body)
}

pub(super) const fn requirement_label(
    requirement: workgraph::CompletionRequirement,
) -> &'static str {
    match requirement {
        workgraph::CompletionRequirement::RevisionOrObservation => {
            "Revision or verified observation"
        }
        workgraph::CompletionRequirement::File => "File artifact",
        workgraph::CompletionRequirement::Observation => "Verified observation",
    }
}

pub(super) const fn evidence_label(kind: workgraph::EvidenceKind) -> &'static str {
    match kind {
        workgraph::EvidenceKind::Revision => "Revision",
        workgraph::EvidenceKind::File => "File",
        workgraph::EvidenceKind::Observation => "Observation",
    }
}
