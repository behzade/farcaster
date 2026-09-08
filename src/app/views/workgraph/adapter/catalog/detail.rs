use gpui::{
    Div, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use workgraph::{PlanOverview, ProjectGraph, WorkStatus};

use super::{WorkGraphBoardView, caption, status_label};
use crate::app::{
    ui::{
        primitives::{ButtonTone, button},
        theme::THEME,
    },
    views::workgraph::{
        contract::PlanData,
        layout::{BoardLayoutMode, DETAIL_WIDTH},
    },
};

impl WorkGraphBoardView {
    pub(super) fn render_catalog_detail(
        &self,
        data: &PlanData,
        row: &PlanOverview,
        layout: BoardLayoutMode,
        entity: Entity<Self>,
    ) -> impl IntoElement {
        let graph = &data.graph;
        let number = row.number;
        let plan = data
            .plans
            .iter()
            .find(|plan| plan.number == number)
            .expect("overview plan");
        let open = entity.clone();
        let back = entity.clone();
        let task = self
            .catalog
            .task
            .and_then(|task| {
                graph
                    .nodes
                    .iter()
                    .find(|node| node.number == task && node.plan_number == number)
            })
            .or_else(|| graph.nodes.iter().find(|node| node.plan_number == number));
        div()
            .id("plan-overview-detail")
            .w(px(DETAIL_WIDTH))
            .flex_none()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .when(layout == BoardLayoutMode::Narrow, |detail| detail.w_full())
            .border_l(THEME.border)
            .border_color(THEME.colors.border)
            .p(THEME.space.md)
            .flex()
            .flex_col()
            .gap(THEME.space.sm)
            .child(button(
                "catalog-detail-back",
                "Close details",
                ButtonTone::Quiet,
                true,
                move |_, cx| {
                    back.update(cx, |this, cx| {
                        this.catalog.selected = None;
                        this.catalog.task = None;
                        cx.notify();
                    })
                },
            ))
            .child(
                div()
                    .text_size(THEME.type_scale.reading)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(plan.title.clone()),
            )
            .child(status_label(row.status))
            .child(caption(format!(
                "{} of {} tasks done · {} blocked",
                row.done, row.total, row.blocked
            )))
            .child(button(
                "catalog-open-plan",
                "Open plan",
                ButtonTone::Neutral,
                true,
                move |window, cx| open.update(cx, |this, cx| this.open_plan(number, window, cx)),
            ))
            .child(caption("Tasks"))
            .child(
                div()
                    .id("catalog-detail-tasks")
                    .max_h(px(240.0))
                    .overflow_y_scroll()
                    .flex_none()
                    .children(
                        graph
                            .nodes
                            .iter()
                            .filter(|node| node.plan_number == number)
                            .map(|node| {
                                let is_selected =
                                    task.is_some_and(|task| task.number == node.number);
                                let task = node.number;
                                let entity = entity.clone();
                                div()
                                    .id(format!("catalog-task-{task}"))
                                    .role(gpui::Role::Button)
                                    .aria_label(format!("Inspect task {}", node.title))
                                    .flex_none()
                                    .cursor_pointer()
                                    .border_b(THEME.border)
                                    .border_color(THEME.colors.border)
                                    .py(THEME.space.sm)
                                    .bg(if is_selected {
                                        THEME.colors.selection
                                    } else {
                                        THEME.colors.panel
                                    })
                                    .hover(|style| style.bg(THEME.colors.hover))
                                    .on_click(move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.catalog.task = Some(task);
                                            cx.notify();
                                        })
                                    })
                                    .child(
                                        div()
                                            .text_size(THEME.type_scale.body_small)
                                            .child(format!("#{task} {}", node.title)),
                                    )
                                    .child(status_label(graph.work_status(task)))
                            }),
                    ),
            )
            .when_some(task, |detail, task| {
                let state = graph.task_state(task.number);
                detail
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(task.title.clone()),
                    )
                    .child(caption("Acceptance"))
                    .child(div().text_size(THEME.type_scale.body_small).child(
                        if task.acceptance.is_empty() {
                            "No acceptance condition recorded.".into()
                        } else {
                            task.acceptance.clone()
                        },
                    ))
                    .when_some(
                        state.as_ref().and_then(|state| state.completion.as_ref()),
                        |detail, completion| {
                            detail
                                .child(caption("Outcome"))
                                .child(
                                    div()
                                        .text_size(THEME.type_scale.body_small)
                                        .child(completion.outcome.note.clone()),
                                )
                                .child(caption(completion.outcome.evidence.reference.clone()))
                        },
                    )
                    .when_some(
                        state.as_ref().and_then(|state| state.owner.as_ref()),
                        |detail, owner| {
                            detail.child(caption(format!("Owner: {}", owner.session_id)))
                        },
                    )
                    .child(caption("Depends on"))
                    .child(render_relations(graph, task.number, true, entity.clone()))
                    .child(caption(
                        if graph.work_status(task.number) == WorkStatus::Done {
                            "Enables"
                        } else {
                            "Next tasks"
                        },
                    ))
                    .child(render_relations(graph, task.number, false, entity.clone()))
            })
            .child(caption("Linked sessions"))
            .when(
                !graph
                    .sessions
                    .iter()
                    .any(|session| session.plan_number == number),
                |detail| detail.child(caption("No linked sessions.")),
            )
            .children(
                graph
                    .sessions
                    .iter()
                    .filter(|session| session.plan_number == number)
                    .map(|session| caption(session.session_id.clone())),
            )
    }
}

fn render_relations(
    graph: &ProjectGraph,
    task: u64,
    incoming: bool,
    entity: Entity<WorkGraphBoardView>,
) -> Div {
    let nodes = graph
        .edges
        .iter()
        .filter_map(|edge| {
            let number = if incoming && edge.to == task {
                edge.from
            } else if !incoming && edge.from == task {
                edge.to
            } else {
                return None;
            };
            graph.nodes.iter().find(|node| node.number == number)
        })
        .collect::<Vec<_>>();
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .when(nodes.is_empty(), |list| list.child(caption("None")))
        .children(nodes.into_iter().map(|node| {
            let entity = entity.clone();
            let number = node.number;
            let plan = node.plan_number;
            button(
                format!("catalog-relation-{incoming}-{number}"),
                format!(
                    "#{} {} · {}",
                    number,
                    node.title,
                    graph.work_status(number).label()
                ),
                ButtonTone::Quiet,
                true,
                move |window, cx| {
                    entity.update(cx, |this, cx| {
                        this.catalog.filter = None;
                        this.catalog.hide_done = false;
                        this.catalog.selected = Some(plan);
                        this.catalog.task = Some(number);
                        this.search
                            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
                        cx.notify();
                    })
                },
            )
            .justify_start()
        }))
}
