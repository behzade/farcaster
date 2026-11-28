use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
};

use gpui::{
    AppContext as _, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Role, StatefulInteractiveElement as _, Styled as _, Task, WeakEntity, div,
    prelude::FluentBuilder as _,
};

use super::{
    components::render_session_goal,
    contract::{PlanData, PlanLoadState, PlanRow},
    core::plan_rows,
};
use crate::{
    app::FarcasterApp,
    app::ui::assets::AppIcon,
    app::ui::primitives::{AppIconSize, ButtonTone, app_icon, button, sidebar_heading},
    app::ui::theme::theme,
};
use workgraph::load_plan;

const MAX_CACHED_SESSIONS: usize = 8;

pub(crate) struct WorkGraphSidebarView {
    app: WeakEntity<FarcasterApp>,
    store: Result<crate::app::persistence::SharedStateStore, String>,
    project: PathBuf,
    session_id: Option<String>,
    session_goal: Option<crate::agents::SessionGoal>,
    state: PlanLoadState,
    cache: HashMap<(PathBuf, String), Box<PlanData>>,
    cache_order: VecDeque<(PathBuf, String)>,
    refresh: Option<Task<()>>,
}

impl WorkGraphSidebarView {
    pub(crate) fn new(
        app: WeakEntity<FarcasterApp>,
        store: Result<crate::app::persistence::SharedStateStore, String>,
        project: PathBuf,
        _cx: &mut gpui::Context<Self>,
    ) -> Self {
        let state = match &store {
            Ok(_) => PlanLoadState::Ready(Box::default()),
            Err(error) => PlanLoadState::Failed(error.clone()),
        };
        Self {
            app,
            store,
            project,
            session_id: None,
            session_goal: None,
            state,
            cache: HashMap::new(),
            cache_order: VecDeque::new(),
            refresh: None,
        }
    }

    pub(crate) fn refresh_for(
        &mut self,
        project: PathBuf,
        session_id: Option<String>,
        session_goal: Option<Option<crate::agents::SessionGoal>>,
        cx: &mut gpui::Context<Self>,
    ) {
        let changed = self.project != project || self.session_id != session_id;
        if changed {
            self.refresh = None;
            self.project = project;
            self.session_id = session_id;
            self.session_goal = None;
            let key = self
                .session_id
                .as_ref()
                .map(|session_id| (self.project.clone(), session_id.clone()));
            let cached = key.as_ref().and_then(|key| self.cache.get(key).cloned());
            if cached.is_some()
                && let Some(key) = key
            {
                self.cache_order.retain(|entry| entry != &key);
                self.cache_order.push_back(key);
            }
            let needs_load = self.session_id.is_some();
            self.state = match (&self.store, cached) {
                (_, Some(data)) => PlanLoadState::Ready(data),
                (Ok(_), None) => PlanLoadState::Ready(Box::default()),
                (Err(error), None) => PlanLoadState::Failed(error.clone()),
            };
            cx.notify();
            if needs_load {
                self.refresh(cx);
            }
        } else {
            self.invalidate_and_refresh(cx);
        }
        if let Some(goal) = session_goal {
            self.set_session_goal(goal, cx);
        }
    }

    pub(crate) fn invalidate_and_refresh(&mut self, cx: &mut gpui::Context<Self>) {
        self.cache.clear();
        self.cache_order.clear();
        self.refresh(cx);
    }

    pub(crate) fn set_session_goal(
        &mut self,
        goal: Option<crate::agents::SessionGoal>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.session_goal != goal {
            self.session_goal = goal;
            cx.notify();
        }
    }

    pub(crate) fn refresh(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(session_id) = self.session_id.clone() else {
            return;
        };
        let store = self.store.clone();
        let project = self.project.clone();
        let key = (project.clone(), session_id.clone());
        let load = cx.background_spawn(async move {
            store?.with(|store| {
                store
                    .with_connection(|connection| load_plan(connection, project, Some(&session_id)))
            })
        });
        self.refresh = Some(cx.spawn(async move |weak, cx| {
            let state = match load.await {
                Ok(data) => PlanLoadState::Ready(Box::new(data)),
                Err(error) => PlanLoadState::Failed(error),
            };
            let _ = weak.update(cx, |this, cx| {
                if this.project != key.0 || this.session_id.as_deref() != Some(&key.1) {
                    return;
                }
                if let PlanLoadState::Ready(data) = &state {
                    this.cache_order.retain(|entry| entry != &key);
                    this.cache_order.push_back(key.clone());
                    this.cache.insert(key, data.clone());
                    if this.cache.len() > MAX_CACHED_SESSIONS
                        && let Some(oldest) = this.cache_order.pop_front()
                    {
                        this.cache.remove(&oldest);
                    }
                }
                if this.state != state {
                    this.state = state;
                    cx.notify();
                }
            });
        }));
    }
}

impl Render for WorkGraphSidebarView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let _timing =
            crate::app::infrastructure::performance::Timing::new("render.workgraph_sidebar");
        let entity = cx.entity().downgrade();
        let title = match &self.state {
            PlanLoadState::Ready(data) => data
                .snapshot
                .as_ref()
                .and_then(|snapshot| {
                    let title = snapshot.plan.title.trim();
                    (!title.is_empty()
                        && !snapshot.nodes.iter().any(|node| node.title.trim() == title))
                    .then(|| format!("Plan · {title}"))
                })
                .unwrap_or_else(|| "Plan".into()),
            PlanLoadState::Failed(_) => "Plan".into(),
        };
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(sidebar_heading(title));
        let visible = sidebar_visible(&self.state, self.session_id.is_some());
        div()
            .debug_selector(|| "run-panel-plan".into())
            .when(!visible, |sidebar| sidebar.hidden())
            .when(visible, |sidebar| {
                sidebar
                    .flex()
                    .flex_col()
                    .gap(theme().space.sm)
                    .child(header)
                    .when_some(self.session_goal.as_ref(), |sidebar, goal| {
                        sidebar.child(render_session_goal(goal, true))
                    })
                    .child(match &self.state {
                        PlanLoadState::Failed(_) => div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(theme().space.sm)
                            .child(
                                div()
                                    .text_size(theme().type_scale.caption)
                                    .text_color(theme().colors.subtle)
                                    .child("Plan unavailable"),
                            )
                            .child(button(
                                "workgraph-sidebar-retry",
                                "Retry",
                                ButtonTone::Quiet,
                                true,
                                move |_, cx| {
                                    if let Some(entity) = entity.upgrade() {
                                        entity.update(cx, |this, cx| this.refresh(cx));
                                    }
                                },
                            ))
                            .into_any_element(),
                        PlanLoadState::Ready(data) => {
                            let rows = data
                                .snapshot
                                .as_ref()
                                .map(|snapshot| plan_rows(snapshot, &data.graph, ""))
                                .unwrap_or_default();
                            let marker_width = plan_marker_width(&rows);
                            div()
                                .flex()
                                .flex_col()
                                .children(rows.into_iter().map(|row| {
                                    render_sidebar_row(row, marker_width, self.app.clone())
                                }))
                                .into_any_element()
                        }
                    })
            })
    }
}

fn sidebar_visible(state: &PlanLoadState, has_session: bool) -> bool {
    has_session && !matches!(state, PlanLoadState::Ready(data) if data.session_link.is_none())
}

fn plan_marker_width(rows: &[PlanRow]) -> gpui::Pixels {
    let digits = rows
        .iter()
        .map(|row| row.node.number.to_string().len())
        .max()
        .unwrap_or_default()
        .max(4);
    theme().size(8.0) * digits as f32
}

fn render_sidebar_row(
    row: PlanRow,
    marker_width: gpui::Pixels,
    app: WeakEntity<FarcasterApp>,
) -> impl IntoElement {
    let number = row.node.number;
    let accessible = format!("Open plan node {}", row.node.title);
    sidebar_row_content(row, marker_width)
        .id(("workgraph-sidebar-node", number))
        .role(Role::Button)
        .aria_label(accessible)
        .tab_index(0)
        .on_mouse_down(
            gpui::MouseButton::Left,
            crate::app::ui::primitives::preserve_pointer_focus,
        )
        .cursor_pointer()
        .hover(|row| row.bg(theme().colors.highlight))
        .on_click(move |_, window, cx| {
            let _ = app.update(cx, |app, cx| {
                app.open_workgraph_node(number, window, cx);
            });
        })
}

fn sidebar_row_content(row: PlanRow, marker_width: gpui::Pixels) -> gpui::Div {
    let number = row.node.number;
    let title_color = if row.reached {
        theme().colors.muted
    } else {
        theme().colors.text
    };
    crate::app::ui::grid::row()
        .py(theme().space.xs)
        .items_start()
        .gap(theme().space.sm)
        .child(
            crate::app::ui::grid::slot(marker_width)
                .h(theme().type_scale.line_body)
                .justify_end()
                .debug_selector(move || format!("plan-marker-{number}"))
                .text_color(if row.reached {
                    theme().colors.success
                } else if row.current {
                    theme().colors.indicator
                } else {
                    theme().colors.subtle
                })
                .when(row.reached, |marker| {
                    marker.child(app_icon(AppIcon::CheckCircle, AppIconSize::Inline))
                })
                .when(!row.reached, |marker| {
                    marker.child(
                        div()
                            .text_size(theme().type_scale.caption)
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
                .gap(theme().size(2.0))
                .child(
                    div()
                        .line_clamp(2)
                        .debug_selector(move || format!("plan-title-{number}"))
                        .text_size(theme().type_scale.body_small)
                        .line_height(theme().type_scale.line_body)
                        .font_weight(if row.current {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(title_color)
                        .child(row.node.title),
                )
                .when(!row.node.files.is_empty(), |content| {
                    content.child(
                        div()
                            .text_size(theme().type_scale.caption)
                            .text_color(theme().colors.subtle)
                            .child(format!("{} path(s)", row.node.files.len())),
                    )
                }),
        )
}

#[cfg(test)]
#[path = "sidebar_tests.rs"]
mod tests;
