//! Selected-node detail presentation for the Work surface.

use gpui::{
    Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};

use super::WorkGraphBoardView;
use crate::{
    app::workgraph::{
        components::{detail_card, detail_label, evidence_label, requirement_label},
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
                    let session_action = snapshot.walk.as_ref().and_then(|walk| {
                        self.active_session.as_ref().map(|_| {
                            let walk = walk.number;
                            let entity = entity.clone();
                            button(
                                format!("workgraph-link-walk-{walk}"),
                                if linked_here {
                                    "Current session attached"
                                } else {
                                    "Attach current session"
                                },
                                if linked_here {
                                    ButtonTone::Quiet
                                } else {
                                    ButtonTone::Neutral
                                },
                                !linked_here,
                                move |_, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.link_active_session(walk, cx);
                                    });
                                },
                            )
                        })
                    });
                    let back = entity.clone();
                    div()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.sm)
                        .when(narrow, |detail| {
                            detail.child(button(
                                "workgraph-detail-back",
                                "Back to plan",
                                ButtonTone::Quiet,
                                true,
                                move |_, cx| {
                                    back.update(cx, |this, cx| this.clear_selection(cx));
                                },
                            ))
                        })
                        .child(
                            detail_card()
                                .child(
                                    div()
                                        .text_size(THEME.type_scale.caption)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(if current {
                                            THEME.colors.accent
                                        } else {
                                            THEME.colors.muted
                                        })
                                        .child(if current {
                                            format!("CURRENT · NODE {}", node.number)
                                        } else {
                                            format!("NODE {}", node.number)
                                        }),
                                )
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
                        .child(
                            detail_card()
                                .child(detail_label("Scoped paths"))
                                .when(node.files.is_empty(), |card| {
                                    card.child(
                                        div()
                                            .text_size(THEME.type_scale.body_small)
                                            .text_color(THEME.colors.subtle)
                                            .child("No paths recorded."),
                                    )
                                })
                                .children(node.files.iter().map(|path| {
                                    div()
                                        .text_size(THEME.type_scale.body_small)
                                        .text_color(THEME.colors.code)
                                        .child(path.clone())
                                })),
                        )
                        .child(
                            detail_card()
                                .child(detail_label("Outcome"))
                                .when_some(outcome, |card, step| {
                                    card.child(
                                        div()
                                            .text_size(THEME.type_scale.body_small)
                                            .line_height(THEME.type_scale.line_body)
                                            .child(step.outcome.note.clone()),
                                    )
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
                                .when(outcome.is_none(), |card| {
                                    card.child(
                                        div()
                                            .text_size(THEME.type_scale.body_small)
                                            .text_color(THEME.colors.subtle)
                                            .child(if current {
                                                "Record one concise outcome to advance."
                                            } else {
                                                "This state has not been reached on the active walk."
                                            }),
                                    )
                                }),
                        )
                        .child(
                            detail_card()
                                .child(detail_label("Next states"))
                                .when(successors.is_empty(), |card| {
                                    card.child(
                                        div()
                                            .text_size(THEME.type_scale.body_small)
                                            .text_color(THEME.colors.subtle)
                                            .child("Leaf outcome"),
                                    )
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
                                })),
                        )
                        .children(session_action)
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
