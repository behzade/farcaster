use gpui::{
    Div, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use gpui_component::input::{Input, InputState, Textarea, TextareaState};

use super::{adapter::WorkGraphBoardView, contract::PlanRow};
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
        .ml(px((row.depth.min(8) * 18) as f32))
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

pub(super) fn render_create(
    title: &Entity<InputState>,
    detail: &Entity<TextareaState>,
    has_plan: bool,
    entity: Entity<WorkGraphBoardView>,
) -> impl IntoElement {
    let submit_title = title.clone();
    let submit_detail = detail.clone();
    let submit = entity.clone();
    let cancel = entity;
    div()
        .id("workgraph-create")
        .w(px(420.0))
        .min_w(px(360.0))
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
                        .child(if has_plan { "NEW NODE" } else { "NEW PLAN" }),
                )
                .child(
                    div()
                        .text_size(THEME.type_scale.body)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if has_plan {
                            "Add the next verifiable state"
                        } else {
                            "Name the outcome and its current state"
                        }),
                ),
        )
        .child(Input::new(title).w_full())
        .child(Textarea::new(detail).w_full())
        .child(
            div()
                .flex()
                .gap(THEME.space.xs)
                .child(button(
                    "workgraph-create-submit",
                    if has_plan { "Add node" } else { "Create plan" },
                    ButtonTone::Neutral,
                    true,
                    move |window, cx| {
                        let title = submit_title.read(cx).value().trim().to_owned();
                        if title.is_empty() {
                            return;
                        }
                        let detail = submit_detail.read(cx).value().trim().to_owned();
                        submit_title.update(cx, |input, cx| {
                            input.set_value(String::new(), window, cx);
                        });
                        submit_detail.update(cx, |input, cx| {
                            input.set_value(String::new(), window, cx);
                        });
                        submit.update(cx, |this, cx| this.submit_create(title, detail, cx));
                    },
                ))
                .child(button(
                    "workgraph-create-cancel",
                    "Cancel",
                    ButtonTone::Quiet,
                    true,
                    move |_, cx| cancel.update(cx, |this, cx| this.cancel_create(cx)),
                )),
        )
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
