mod detail;

use gpui::{
    Context, Div, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use workgraph::{PlanOverview, WorkStatus};

use super::{WorkGraphBoardView, render_board_header};
use crate::app::{
    ui::{
        primitives::{ButtonTone, button},
        theme::THEME,
    },
    views::workgraph::{contract::PlanData, layout::BoardLayoutMode},
};

#[derive(Default)]
pub(super) struct CatalogState {
    pub selected: Option<u64>,
    task: Option<u64>,
    filter: Option<WorkStatus>,
    recent: bool,
    hide_done: bool,
}

impl CatalogState {
    fn shows(&self, row: &PlanOverview) -> bool {
        !(self.hide_done && self.filter.is_none() && row.status == WorkStatus::Done)
    }
}

fn status_label(status: WorkStatus) -> Div {
    div()
        .text_size(THEME.type_scale.caption)
        .text_color(match status {
            WorkStatus::Done => THEME.colors.success,
            WorkStatus::Active => THEME.colors.accent,
            WorkStatus::Blocked => THEME.colors.warning,
            WorkStatus::Ready => THEME.colors.muted,
        })
        .child(if status == WorkStatus::Done {
            "✓ Done"
        } else {
            status.label()
        })
}

fn caption(text: impl Into<gpui::SharedString>) -> Div {
    div()
        .text_size(THEME.type_scale.caption)
        .text_color(THEME.colors.subtle)
        .child(text.into())
}

fn date_label(timestamp: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(timestamp.div_euclid(1000))
        .map(|date| {
            format!(
                "{}-{:02}-{:02}",
                date.year(),
                u8::from(date.month()),
                date.day()
            )
        })
        .unwrap_or_else(|_| "—".into())
}

impl WorkGraphBoardView {
    fn catalog_rows(
        &self,
        data: &PlanData,
        summaries: &[PlanOverview],
        cx: &Context<Self>,
    ) -> Vec<PlanOverview> {
        let query = self.search.read(cx).value().trim().to_lowercase();
        let mut rows = summaries.to_vec();
        rows.retain(|row| {
            self.catalog
                .filter
                .is_none_or(|filter| row.status == filter)
                && data
                    .plans
                    .iter()
                    .find(|plan| plan.number == row.number)
                    .is_some_and(|plan| {
                        format!("#{} {} {}", plan.number, plan.title, plan.project)
                            .to_lowercase()
                            .contains(&query)
                            || data.graph.nodes.iter().any(|node| {
                                node.plan_number == plan.number
                                    && format!("{} {}", node.title, node.acceptance)
                                        .to_lowercase()
                                        .contains(&query)
                            })
                    })
        });
        rows.sort_by_key(|row| {
            (
                if self.catalog.recent {
                    WorkStatus::Ready
                } else {
                    row.status
                },
                std::cmp::Reverse(row.updated_at),
                std::cmp::Reverse(row.number),
            )
        });
        rows
    }

    pub(super) fn move_catalog_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let super::PlanLoadState::Ready(data) = &self.state else {
            return;
        };
        let summaries = data.graph.plan_overviews();
        let rows = self
            .catalog_rows(data, &summaries, cx)
            .into_iter()
            .filter(|row| self.catalog.shows(row))
            .collect::<Vec<_>>();
        if rows.is_empty() {
            return;
        }
        let current = self
            .catalog
            .selected
            .and_then(|number| rows.iter().position(|row| row.number == number))
            .unwrap_or(if delta < 0 { 0 } else { rows.len() - 1 });
        self.catalog.selected =
            Some(rows[(current as isize + delta).rem_euclid(rows.len() as isize) as usize].number);
        self.catalog.task = None;
        cx.notify();
    }

    pub(super) fn render_catalog(
        &self,
        data: &PlanData,
        layout: BoardLayoutMode,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let entity = cx.entity();
        let summaries = data.graph.plan_overviews();
        let rows = self.catalog_rows(data, &summaries, cx);
        let selected = self
            .catalog
            .selected
            .and_then(|number| rows.iter().find(|row| row.number == number));
        let narrow_detail = layout == BoardLayoutMode::Narrow && selected.is_some();
        let sort = entity.clone();
        let collapse = entity.clone();
        let project = self.project.file_name().map_or_else(
            || self.project.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let toolbar = div()
            .flex_none()
            .px(THEME.space.md)
            .py(THEME.space.sm)
            .flex()
            .flex_wrap()
            .items_center()
            .gap(THEME.space.xs)
            .children(
                [
                    (None, "All"),
                    (Some(WorkStatus::Active), "Active"),
                    (Some(WorkStatus::Blocked), "Blocked"),
                    (Some(WorkStatus::Ready), "Ready"),
                    (Some(WorkStatus::Done), "Done"),
                ]
                .into_iter()
                .map(|(filter, label)| {
                    let count = summaries
                        .iter()
                        .filter(|row| filter.is_none_or(|filter| filter == row.status))
                        .count();
                    let entity = entity.clone();
                    button(
                        format!("plan-filter-{label}"),
                        format!("{label} {count}"),
                        if self.catalog.filter == filter {
                            ButtonTone::Neutral
                        } else {
                            ButtonTone::Quiet
                        },
                        true,
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.catalog.filter = filter;
                                this.catalog.selected = None;
                                this.catalog.task = None;
                                cx.notify();
                            })
                        },
                    )
                }),
            )
            .child(div().flex_1())
            .child(button(
                "plans-sort",
                if self.catalog.recent {
                    "Sort: Recent"
                } else {
                    "Sort: Status"
                },
                ButtonTone::Quiet,
                true,
                move |_, cx| {
                    sort.update(cx, |this, cx| {
                        this.catalog.recent = !this.catalog.recent;
                        cx.notify();
                    })
                },
            ));
        let list = div()
            .id("workgraph-all-plans")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .px(THEME.space.md)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap(THEME.space.sm)
                    .py(THEME.space.xs)
                    .child(caption("Plan / dependencies").flex_1())
                    .child(caption("Status / progress").w(px(120.0))),
            )
            .when(rows.is_empty(), |list| {
                list.child(
                    caption(if data.plans.is_empty() {
                        "No plans yet. Create a plan to get started."
                    } else {
                        "No plans match this filter and search."
                    })
                    .py(THEME.space.md),
                )
            })
            .when(
                self.catalog.filter.is_none()
                    && summaries.iter().any(|row| row.status == WorkStatus::Done),
                |list| {
                    list.child(button(
                        "plans-toggle-done",
                        if self.catalog.hide_done {
                            "Show completed plans"
                        } else {
                            "Hide completed plans"
                        },
                        ButtonTone::Quiet,
                        true,
                        move |_, cx| {
                            collapse.update(cx, |this, cx| {
                                this.catalog.hide_done = !this.catalog.hide_done;
                                if this.catalog.hide_done {
                                    this.catalog.selected = None;
                                }
                                cx.notify();
                            })
                        },
                    ))
                },
            )
            .children(
                rows.iter()
                    .filter(|row| self.catalog.shows(row))
                    .map(|row| self.render_catalog_row(data, row, entity.clone())),
            );
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(render_board_header(
                "All plans",
                false,
                true,
                &self.search,
                entity.clone(),
            ))
            .child(toolbar)
            .child(
                div()
                    .px(THEME.space.md)
                    .pb(THEME.space.sm)
                    .child(caption(format!("{project} · {} plans", rows.len()))),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .when(!narrow_detail, |body| body.child(list))
                    .when_some(selected, |body, row| {
                        body.child(self.render_catalog_detail(data, row, layout, entity.clone()))
                    }),
            )
            .into_any_element()
    }

    fn render_catalog_row(
        &self,
        data: &PlanData,
        row: &PlanOverview,
        entity: Entity<Self>,
    ) -> impl IntoElement {
        let number = row.number;
        let plan = data
            .plans
            .iter()
            .find(|plan| plan.number == number)
            .expect("overview plan");
        let blockers = data
            .graph
            .edges
            .iter()
            .filter(|edge| {
                data.graph
                    .nodes
                    .iter()
                    .any(|node| node.number == edge.to && node.plan_number == number)
                    && data.graph.work_status(edge.to) != WorkStatus::Done
                    && data.graph.work_status(edge.from) != WorkStatus::Done
            })
            .filter_map(|edge| {
                data.graph
                    .nodes
                    .iter()
                    .find(|node| node.number == edge.from)
            })
            .map(|node| format!("#{} {}", node.number, node.title))
            .collect::<std::collections::BTreeSet<_>>();
        let dependency = if blockers.is_empty() {
            "No pending dependencies".into()
        } else {
            format!(
                "Waiting on {}",
                blockers.into_iter().collect::<Vec<_>>().join(", ")
            )
        };
        div()
            .id(format!("workgraph-plan-{number}"))
            .role(gpui::Role::Button)
            .aria_label(format!("Inspect plan {}", plan.title))
            .flex_none()
            .cursor_pointer()
            .on_mouse_down(
                gpui::MouseButton::Left,
                crate::app::ui::primitives::preserve_pointer_focus,
            )
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.catalog.selected = Some(number);
                    this.catalog.task = None;
                    cx.notify();
                })
            })
            .border_b(THEME.border)
            .border_color(THEME.colors.border)
            .bg(if self.catalog.selected == Some(number) {
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
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(
                        div()
                            .text_size(THEME.type_scale.body_small)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(plan.title.clone()),
                    )
                    .child(caption(dependency).line_clamp(2))
                    .child(caption(format!(
                        "#{} · Updated {}",
                        number,
                        date_label(row.updated_at)
                    ))),
            )
            .child(
                div()
                    .w(px(120.0))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(status_label(row.status))
                    .child(caption(format!("{} / {} done", row.done, row.total))),
            )
    }
}
