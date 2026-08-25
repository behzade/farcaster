//! Selected-node detail presentation for the Work surface.

use gpui::{
    Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};

use super::WorkGraphBoardView;
use crate::{
    app::workgraph::{
        components::{
            detail_action, detail_copy, detail_empty, detail_rule, detail_section, evidence_label,
            requirement_label,
        },
        contract::{PlanData, PlanLoadState},
        core::active_outcome,
        layout::{BoardLayoutMode, DETAIL_MIN_WIDTH, DETAIL_WIDTH},
    },
    primitives::{ButtonTone, FeedbackTone, button, feedback},
    theme::THEME,
};

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
            .p(THEME.space.md)
            .bg(THEME.colors.panel)
            .when(!external, |detail| {
                detail
                    .border_l(THEME.border)
                    .border_color(THEME.colors.border)
            })
            .child(match (snapshot, node) {
                (Some(snapshot), Some(node)) => {
                    let outcome = active_outcome(snapshot, node.number);
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
                    let back = entity.clone();
                    let add_successor = entity.clone();
                    let leaf = successors.is_empty();
                    div()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.md)
                        .when(narrow, |detail| {
                            detail.child(detail_action(button(
                                "workgraph-detail-back",
                                "Back to plan",
                                ButtonTone::Quiet,
                                true,
                                move |_, cx| {
                                    back.update(cx, |this, cx| this.clear_selection(cx));
                                },
                            )))
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(THEME.space.sm)
                                .child(div().flex().items_center().gap(THEME.space.xs).when(
                                    current,
                                    |status| {
                                        status
                                            .child(
                                                div()
                                                    .size(px(8.0))
                                                    .rounded_full()
                                                    .bg(THEME.colors.accent),
                                            )
                                            .child(
                                                div()
                                                    .text_size(THEME.type_scale.caption)
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(THEME.colors.accent)
                                                    .child("CURRENT"),
                                            )
                                    },
                                ))
                                .child(
                                    div()
                                        .text_size(THEME.type_scale.caption)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(THEME.colors.muted)
                                        .child(format!(
                                            "NODE {}{}",
                                            node.number,
                                            if leaf { " · LEAF" } else { "" }
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(THEME.space.xs)
                                .child(
                                    div()
                                        .text_size(THEME.type_scale.display)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .line_height(THEME.type_scale.line_composer)
                                        .child(node.title.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(THEME.type_scale.caption)
                                        .text_color(THEME.colors.subtle)
                                        .child(requirement_label(node.completion)),
                                ),
                        )
                        .child(detail_rule())
                        .child(
                            detail_section("ACCEPTANCE").child(
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
                            ),
                        )
                        .child(
                            detail_section("SCOPED PATHS")
                                .when(node.files.is_empty(), |section| {
                                    section.child(detail_empty("No paths recorded."))
                                })
                                .children(node.files.iter().map(|path| {
                                    div()
                                        .text_size(THEME.type_scale.body_small)
                                        .text_color(THEME.colors.code)
                                        .child(path.clone())
                                })),
                        )
                        .child(detail_rule())
                        .child(
                            detail_section("OUTCOME")
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
                                    section.child(detail_empty(if current {
                                        "Record one concise outcome to advance."
                                    } else {
                                        "This state has not been reached on the active walk."
                                    }))
                                }),
                        )
                        .child(detail_rule())
                        .child(
                            detail_section("NEXT STATES")
                                .when(leaf, |section| {
                                    section.child(detail_empty(
                                        "Leaf — completing this node ends the branch",
                                    ))
                                })
                                .children(successors.into_iter().map(|successor| {
                                    let number = successor.number;
                                    let entity = entity.clone();
                                    div()
                                        .id(format!("workgraph-successor-{number}"))
                                        .cursor_pointer()
                                        .rounded(THEME.radius)
                                        .px(THEME.space.xs)
                                        .py(THEME.space.xs)
                                        .hover(|row| row.bg(THEME.colors.hover))
                                        .on_click(move |_, _, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.select_node(number, cx);
                                            });
                                        })
                                        .text_size(THEME.type_scale.body_small)
                                        .child(successor.title)
                                }))
                                .child(detail_action(button(
                                    "workgraph-detail-add-successor",
                                    "Add successor",
                                    ButtonTone::Quiet,
                                    true,
                                    move |window, cx| {
                                        add_successor.update(cx, |this, cx| {
                                            this.start_create(window, cx);
                                        });
                                    },
                                ))),
                        )
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
