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
    assets::AppIcon,
    primitives::{AppIconSize, ButtonTone, app_icon, button},
    theme::THEME,
};

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
        .cursor_pointer()
        .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.select_node(number, cx)))
        .border_l(px(if row.current { 3.0 } else { 1.0 }))
        .border_color(if row.current {
            THEME.colors.accent
        } else {
            THEME.colors.border
        })
        .bg(if is_selected {
            THEME.colors.surface
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

pub(super) fn render_create_step(
    title: &Entity<InputState>,
    detail: &Entity<TextareaState>,
    stage: CreateStage,
    current_state_complete: bool,
    can_submit: bool,
    entity: Entity<WorkGraphBoardView>,
) -> impl IntoElement {
    let add_node = stage == CreateStage::Node;
    let show_outcome = stage == CreateStage::Outcome;
    let cancel = entity.clone();
    let back = entity.clone();
    let next = entity.clone();
    let submit = entity;

    div()
        .size_full()
        .min_h_0()
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(52.0))
                .flex_none()
                .px(THEME.space.md)
                .flex()
                .items_center()
                .justify_between()
                .border_b(THEME.border)
                .border_color(THEME.colors.border)
                .child(
                    div()
                        .text_size(THEME.type_scale.body)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if add_node { "Add node" } else { "New plan" }),
                )
                .when(!add_node, |header| {
                    header.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(THEME.space.xs)
                            .child(step_marker("Current state", !show_outcome))
                            .child(app_icon(AppIcon::CaretRight, AppIconSize::Inline))
                            .child(step_marker("Outcome", show_outcome)),
                    )
                }),
        )
        .child(
            div()
                .id("workgraph-create-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(THEME.space.md)
                .py(THEME.space.md)
                .flex()
                .justify_center()
                .child(
                    div()
                        .w_full()
                        .max_w(px(520.0))
                        .when(add_node, |form| {
                            form.child(compact_field("Node", Input::new(title).w_full()))
                                .child(div().mt(THEME.space.md).child(compact_field(
                                    "Paths (optional)",
                                    Textarea::new(detail).w_full().appearance(true),
                                )))
                        })
                        .when(stage == CreateStage::CurrentState, |form| {
                            form.child(compact_field(
                                "Current state",
                                Textarea::new(detail).w_full().appearance(true),
                            ))
                        })
                        .when(show_outcome, |form| {
                            form.child(compact_field("Outcome", Input::new(title).w_full()))
                        }),
                ),
        )
        .child(
            div()
                .h(px(56.0))
                .flex_none()
                .px(THEME.space.md)
                .flex()
                .items_center()
                .justify_between()
                .border_t(THEME.border)
                .border_color(THEME.colors.border)
                .child(if show_outcome {
                    button(
                        "workgraph-create-back",
                        "Back",
                        ButtonTone::Quiet,
                        true,
                        move |window, cx| {
                            back.update(cx, |this, cx| {
                                this.previous_create_step(window, cx);
                            });
                        },
                    )
                } else {
                    button(
                        "workgraph-create-cancel",
                        "Cancel",
                        ButtonTone::Quiet,
                        true,
                        move |window, cx| {
                            cancel.update(cx, |this, cx| {
                                this.cancel_create(window, cx);
                            });
                        },
                    )
                })
                .child(if stage == CreateStage::CurrentState {
                    button(
                        "workgraph-create-next",
                        "Next",
                        ButtonTone::Accent,
                        current_state_complete,
                        move |window, cx| {
                            next.update(cx, |this, cx| {
                                this.next_create_step(window, cx);
                            });
                        },
                    )
                } else {
                    button(
                        "workgraph-create-submit",
                        if add_node { "Add node" } else { "Create plan" },
                        ButtonTone::Accent,
                        can_submit,
                        move |window, cx| {
                            submit.update(cx, |this, cx| {
                                this.submit_create_inputs(window, cx);
                            });
                        },
                    )
                }),
        )
}

fn step_marker(label: &'static str, active: bool) -> impl IntoElement {
    div()
        .px(THEME.space.xs)
        .py(px(2.0))
        .rounded(THEME.radius)
        .text_size(THEME.type_scale.caption)
        .text_color(if active {
            THEME.colors.text
        } else {
            THEME.colors.subtle
        })
        .when(active, |marker| marker.bg(THEME.colors.surface))
        .child(label)
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

pub(super) fn detail_card() -> Div {
    div()
        .p(THEME.space.sm)
        .rounded(THEME.radius)
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.canvas)
        .flex()
        .flex_col()
        .gap(THEME.space.sm)
}

pub(super) fn detail_label(label: &'static str) -> Div {
    div()
        .text_size(THEME.type_scale.caption)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(THEME.colors.muted)
        .child(label)
}

pub(super) const fn requirement_label(
    requirement: workgraph::contract::CompletionRequirement,
) -> &'static str {
    match requirement {
        workgraph::contract::CompletionRequirement::RevisionOrObservation => {
            "Revision or verified observation"
        }
        workgraph::contract::CompletionRequirement::File => "File artifact",
        workgraph::contract::CompletionRequirement::Observation => "Verified observation",
    }
}

pub(super) const fn evidence_label(kind: workgraph::contract::EvidenceKind) -> &'static str {
    match kind {
        workgraph::contract::EvidenceKind::Revision => "Revision",
        workgraph::contract::EvidenceKind::File => "File",
        workgraph::contract::EvidenceKind::Observation => "Observation",
    }
}
