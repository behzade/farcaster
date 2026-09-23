use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
};

use gpui::{
    AppContext as _, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Role, StatefulInteractiveElement as _, Styled as _, Task, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};

use super::{
    components::render_session_goal,
    contract::{PlanData, PlanLoadState, PlanRow},
    core::plan_rows,
};
use crate::{
    app::FarcasterApp,
    app::ui::assets::AppIcon,
    app::ui::primitives::{
        AppIconSize, ButtonTone, FeedbackTone, app_icon, button, feedback, section_heading,
    },
    app::ui::theme::THEME,
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
            let needs_load = self.session_id.is_some() && cached.is_none();
            self.state = match (&self.store, cached) {
                (_, Some(data)) => PlanLoadState::Ready(data),
                (Ok(_), None) => PlanLoadState::Ready(Box::default()),
                (Err(error), None) => PlanLoadState::Failed(error.clone()),
            };
            cx.notify();
            if needs_load {
                self.refresh(cx);
            }
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
        let notify_loading = prepare_refresh(&mut self.state);
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
        if notify_loading {
            cx.notify();
        }
    }
}

fn prepare_refresh(state: &mut PlanLoadState) -> bool {
    if matches!(state, PlanLoadState::Ready(_) | PlanLoadState::Loading) {
        return false;
    }
    *state = PlanLoadState::Loading;
    true
}

impl Render for WorkGraphSidebarView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let _timing =
            crate::app::infrastructure::performance::Timing::new("render.workgraph_sidebar");
        let entity = cx.entity().downgrade();
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(section_heading("Current plan"));
        let visible = sidebar_visible(&self.state, self.session_id.is_some());
        div()
            .when(!visible, |sidebar| sidebar.hidden())
            .when(visible, |sidebar| {
                sidebar
                    .flex()
                    .flex_col()
                    .gap(px(11.0))
                    .child(header)
                    .when_some(self.session_goal.as_ref(), |sidebar, goal| {
                        sidebar.child(render_session_goal(goal, true))
                    })
                    .child(match &self.state {
                        PlanLoadState::Loading => feedback(
                            "workgraph-sidebar-loading",
                            "Loading plan…",
                            FeedbackTone::Info,
                        )
                        .into_any_element(),
                        PlanLoadState::Failed(_) => div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(THEME.space.sm)
                            .child(
                                div()
                                    .text_size(THEME.type_scale.caption)
                                    .text_color(THEME.colors.subtle)
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
                        PlanLoadState::Ready(data) => div()
                            .flex()
                            .flex_col()
                            .children(
                                data.snapshot
                                    .as_ref()
                                    .map(|snapshot| plan_rows(snapshot, &data.graph, ""))
                                    .unwrap_or_default()
                                    .into_iter()
                                    .map(|row| render_sidebar_row(row, self.app.clone())),
                            )
                            .into_any_element(),
                    })
            })
    }
}

fn sidebar_visible(state: &PlanLoadState, has_session: bool) -> bool {
    has_session && !matches!(state, PlanLoadState::Ready(data) if data.session_link.is_none())
}

fn render_sidebar_row(row: PlanRow, app: WeakEntity<FarcasterApp>) -> impl IntoElement {
    let number = row.node.number;
    let title_color = if row.reached {
        THEME.colors.subtle
    } else {
        THEME.colors.text
    };
    div()
        .id(("workgraph-sidebar-node", number))
        .role(Role::Button)
        .aria_label(format!("Open plan node {}", row.node.title))
        .tab_index(0)
        .on_mouse_down(
            gpui::MouseButton::Left,
            crate::app::ui::primitives::preserve_pointer_focus,
        )
        .cursor_pointer()
        .px(px(2.0))
        .py(px(3.0))
        .flex()
        .items_start()
        .gap(px(7.0))
        .hover(|row| row.bg(THEME.colors.surface))
        .on_click(move |_, window, cx| {
            let _ = app.update(cx, |app, cx| {
                app.open_workgraph_node(number, window, cx);
            });
        })
        .child(
            div()
                .w(px(16.0))
                .h(px(20.0))
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
                .gap(px(2.0))
                .child(
                    div()
                        .line_clamp(2)
                        .text_size(THEME.type_scale.body_small)
                        .font_weight(if row.current {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(title_color)
                        .when(row.reached, |title| title.line_through())
                        .child(row.node.title),
                )
                .when(!row.node.files.is_empty(), |content| {
                    content.child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(format!("{} path(s)", row.node.files.len())),
                    )
                }),
        )
}

#[cfg(test)]
#[path = "sidebar_tests.rs"]
mod tests;
