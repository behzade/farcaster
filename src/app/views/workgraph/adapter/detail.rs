use gpui::{
    Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};

use super::WorkGraphBoardView;
use crate::{
    app::ui::primitives::{ButtonTone, FeedbackTone, button, feedback},
    app::ui::theme::{MONO_FONT_FAMILY, THEME},
    app::views::workgraph::{
        components::{
            detail_action, detail_copy, detail_empty, detail_rule, detail_section, evidence_label,
            requirement_label,
        },
        contract::{PlanData, PlanLoadState},
        layout::{BoardLayoutMode, DETAIL_MIN_WIDTH, DETAIL_WIDTH},
    },
};

fn render_node_identity(node: &workgraph::Node, current: bool, leaf: bool) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.sm)
        .child(
            div()
                .flex()
                .items_center()
                .gap(THEME.space.sm)
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.muted)
                .child(
                    div()
                        .font_family(MONO_FONT_FAMILY)
                        .child(format!("#{:02}", node.number)),
                )
                .when(current, |meta| {
                    meta.child(
                        div()
                            .px(THEME.space.xs)
                            .rounded(THEME.radius)
                            .bg(THEME.colors.session_selection)
                            .text_color(THEME.colors.accent)
                            .child("Current"),
                    )
                })
                .when(leaf, |meta| meta.child("End of branch")),
        )
        .child(
            div()
                .text_size(THEME.type_scale.display)
                .font_weight(FontWeight::SEMIBOLD)
                .line_height(THEME.type_scale.line_composer)
                .child(node.title.clone()),
        )
}

fn render_acceptance(node: &workgraph::Node) -> impl IntoElement {
    detail_section("Acceptance").child(
        detail_copy()
            .text_color(if node.acceptance.is_empty() {
                THEME.colors.subtle
            } else {
                THEME.colors.text
            })
            .child(if node.acceptance.is_empty() {
                "No acceptance condition recorded.".to_owned()
            } else {
                node.acceptance.clone()
            }),
    )
}

fn render_scoped_paths(node: &workgraph::Node) -> impl IntoElement {
    detail_section("Files").children(node.files.iter().map(|path| {
        div()
            .text_size(THEME.type_scale.body_small)
            .font_family(MONO_FONT_FAMILY)
            .text_color(THEME.colors.code)
            .child(path.clone())
    }))
}

fn render_outcome(
    completion: workgraph::CompletionRequirement,
    outcome: Option<&workgraph::WalkStep>,
    current: bool,
) -> impl IntoElement {
    detail_section("Outcome")
        .p(THEME.space.sm)
        .border_l(px(2.0))
        .border_color(if current {
            THEME.colors.accent
        } else {
            THEME.colors.border
        })
        .bg(THEME.colors.surface)
        .when_some(outcome, |section, step| {
            section
                .child(detail_copy().child(step.outcome.note.clone()))
                .child(
                    div()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child(format!(
                            "{} · {}",
                            evidence_label(step.outcome.evidence.kind),
                            step.outcome.evidence.reference
                        )),
                )
        })
        .when(outcome.is_none(), |section| {
            section
                .child(detail_empty(if current {
                    "Awaiting an outcome to advance."
                } else {
                    "Not reached on this walk."
                }))
                .child(
                    div()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.muted)
                        .child(format!("Evidence: {}", requirement_label(completion))),
                )
        })
}

fn render_successors(
    successors: Vec<workgraph::Node>,
    entity: Entity<WorkGraphBoardView>,
) -> impl IntoElement {
    let add_successor = entity.clone();
    let leaf = successors.is_empty();
    detail_section("Next steps")
        .when(leaf, |section| {
            section.child(detail_empty("Completing this node ends the branch."))
        })
        .children(successors.into_iter().map(|successor| {
            let number = successor.number;
            let entity = entity.clone();
            button(
                format!("workgraph-successor-{number}"),
                format!("#{}  {} →", number, successor.title),
                ButtonTone::Quiet,
                true,
                move |_, cx| {
                    entity.update(cx, |this, cx| this.select_node(number, cx));
                },
            )
        }))
        .child(detail_action(button(
            "workgraph-detail-add-successor",
            "Add successor",
            ButtonTone::Neutral,
            true,
            move |window, cx| {
                add_successor.update(cx, |this, cx| this.start_create(window, cx));
            },
        )))
}

impl WorkGraphBoardView {
    pub(super) fn render_detail(
        &self,
        entity: Entity<Self>,
        data: &PlanData,
        layout: BoardLayoutMode,
        external: bool,
    ) -> impl IntoElement {
        let snapshot = data.snapshot.as_ref();
        let node = snapshot.and_then(|snapshot| {
            self.selected
                .and_then(|number| snapshot.nodes.iter().find(|node| node.number == number))
        });
        let narrow = layout == BoardLayoutMode::Narrow;
        div()
            .id("workgraph-node-detail")
            .when(!external, |detail| {
                detail.w(px(DETAIL_WIDTH)).min_w(px(DETAIL_MIN_WIDTH))
            })
            .when(narrow || external, |detail| detail.w_full().min_w_0())
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .px(THEME.space.md)
            .py(THEME.space.md)
            .bg(THEME.colors.panel)
            .when(!external && !narrow, |detail| {
                detail
                    .border_l(THEME.border)
                    .border_color(THEME.colors.surface)
            })
            .child(match (snapshot, node) {
                (Some(snapshot), Some(node)) => {
                    let outcome = snapshot.active_outcome(node.number);
                    let current = snapshot
                        .walk
                        .as_ref()
                        .is_some_and(|walk| walk.current_node == Some(node.number));
                    let successors = snapshot
                        .edges
                        .iter()
                        .filter(|edge| edge.from == node.number)
                        .filter_map(|edge| {
                            snapshot.nodes.iter().find(|node| node.number == edge.to)
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    let linked_here = data.session_link.as_ref().is_some_and(|link| {
                        snapshot
                            .walk
                            .as_ref()
                            .is_some_and(|walk| link.walk_number == walk.number)
                    });
                    let session_action = match (&snapshot.walk, &self.active_session) {
                        (Some(walk), Some(_)) if !linked_here => {
                            let walk = walk.number;
                            let entity = entity.clone();
                            Some(button(
                                format!("workgraph-link-walk-{walk}"),
                                "Attach current session",
                                ButtonTone::Quiet,
                                true,
                                move |_, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.link_active_session(walk, cx);
                                    });
                                },
                            ))
                        }
                        _ => None,
                    };
                    let leaf = successors.is_empty();
                    div()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.md)
                        .w_full()
                        .max_w(px(620.0))
                        .mx_auto()
                        .child(render_node_identity(node, current, leaf))
                        .child(render_acceptance(node))
                        .when(!node.files.is_empty(), |detail| {
                            detail.child(render_scoped_paths(node))
                        })
                        .child(render_outcome(node.completion, outcome, current))
                        .child(detail_rule())
                        .child(render_successors(successors, entity))
                        .when_some(session_action, |detail, action| {
                            detail.child(detail_action(action))
                        })
                        .into_any_element()
                }
                _ => div()
                    .id("workgraph-detail-empty")
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(THEME.space.xs)
                    .child(
                        div()
                            .text_size(THEME.type_scale.body)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("No node selected"),
                    )
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child("Choose a node to inspect its scope and outcome."),
                    )
                    .into_any_element(),
            })
    }

    pub(crate) fn render_external_detail(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let entity = cx.entity();
        match &self.state {
            PlanLoadState::Loading => feedback(
                "workgraph-detail-loading",
                "Loading node…",
                FeedbackTone::Info,
            )
            .into_any_element(),
            PlanLoadState::Failed(error) => {
                feedback("workgraph-detail-error", error.clone(), FeedbackTone::Error)
                    .into_any_element()
            }
            PlanLoadState::Ready(data) => self
                .render_detail(entity, data, BoardLayoutMode::Wide, true)
                .into_any_element(),
        }
    }
}
