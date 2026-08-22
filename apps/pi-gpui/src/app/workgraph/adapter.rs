//! GPUI and SQLite adapters for the workgraph plan list.

mod detail;

use std::path::PathBuf;

use super::{
    components::{render_create, render_plan_list},
    contract::PlanLoadState,
    core::{adjacent_node_number, plan_rows},
    layout::{BoardLayoutMode, board_layout_mode},
    persistence::{add_node, create_plan, link_session, load_plan},
};
use crate::{
    primitives::{ButtonTone, FeedbackTone, button, feedback},
    theme::THEME,
};
use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    Task, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::input::{Input, InputEvent, InputState, TextareaState};

pub(crate) const WORKGRAPH_KEY_CONTEXT: &str = "PiWorkGraph";
pub(crate) const WORKGRAPH_NAV_KEY_CONTEXT: &str = "PiWorkGraph && !Input";

pub(crate) struct WorkGraphBoardView {
    database: PathBuf,
    project: PathBuf,
    pub(super) state: PlanLoadState,
    focus: FocusHandle,
    pub(super) selected: Option<u64>,
    creating: bool,
    pub(super) active_session: Option<(String, String)>,
    search: Option<Entity<InputState>>,
    create_title: Option<Entity<InputState>>,
    create_detail: Option<Entity<TextareaState>>,
    refresh: Option<Task<()>>,
    subscriptions: Vec<Subscription>,
}

impl WorkGraphBoardView {
    pub(crate) fn new(
        database: Result<PathBuf, String>,
        project: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        let (database, state) = match database {
            Ok(database) => (database, PlanLoadState::Loading),
            Err(error) => (PathBuf::new(), PlanLoadState::Failed(error)),
        };
        let should_refresh = matches!(state, PlanLoadState::Loading);
        let mut view = Self {
            database,
            project,
            state,
            focus: cx.focus_handle(),
            selected: None,
            creating: false,
            active_session: None,
            search: None,
            create_title: None,
            create_detail: None,
            refresh: None,
            subscriptions: Vec::new(),
        };
        if should_refresh {
            view.refresh(cx);
        }
        view
    }

    pub(crate) fn refresh_for(
        &mut self,
        project: PathBuf,
        active_session: Option<(String, String)>,
        cx: &mut Context<Self>,
    ) {
        self.project = project;
        self.active_session = active_session;
        self.refresh(cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.state = PlanLoadState::Loading;
        let database = self.database.clone();
        let project = self.project.clone();
        let session_id = self.active_session.as_ref().map(|(id, _)| id.clone());
        let load =
            cx.background_spawn(async move { load_plan(database, project, session_id.as_deref()) });
        self.refresh = Some(cx.spawn(async move |weak, cx| {
            let state = match load.await {
                Ok(data) => PlanLoadState::Ready(Box::new(data)),
                Err(error) => PlanLoadState::Failed(error),
            };
            let _ = weak.update(cx, |this, cx| {
                this.state = state;
                if let PlanLoadState::Ready(data) = &this.state
                    && let Some(snapshot) = &data.snapshot
                    && !snapshot
                        .nodes
                        .iter()
                        .any(|node| Some(node.number) == this.selected)
                {
                    this.selected = snapshot
                        .walk
                        .as_ref()
                        .and_then(|walk| walk.current_node)
                        .or(Some(snapshot.plan.root_node));
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(crate) fn select_node(&mut self, number: u64, cx: &mut Context<Self>) {
        if self.selected != Some(number) || self.creating {
            self.selected = Some(number);
            self.creating = false;
            cx.notify();
        }
    }

    pub(crate) fn select_issue(&mut self, number: u64, cx: &mut Context<Self>) {
        self.select_node(number, cx);
    }

    pub(super) fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.selected.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
    }

    pub(crate) fn focus_search(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = &self.search {
            search.read(cx).focus_handle(cx).focus(window, cx);
        } else {
            self.focus.focus(window, cx);
        }
    }

    pub(crate) fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let PlanLoadState::Ready(data) = &self.state else {
            return;
        };
        let Some(snapshot) = &data.snapshot else {
            return;
        };
        let search = self
            .search
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();
        let rows = plan_rows(snapshot, &search);
        if let Some(number) = adjacent_node_number(&rows, self.selected, delta) {
            self.select_node(number, cx);
        }
    }

    pub(crate) fn dismiss_work_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.creating {
            self.creating = false;
            cx.notify();
            return;
        }
        if let Some(search) = &self.search
            && !search.read(cx).value().is_empty()
        {
            search.update(cx, |input, cx| {
                input.set_value(String::new(), window, cx);
            });
            return;
        }
        if self.selected.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn start_create(&mut self, cx: &mut Context<Self>) {
        if !self.creating {
            self.creating = true;
            cx.notify();
        }
    }

    pub(super) fn cancel_create(&mut self, cx: &mut Context<Self>) {
        if self.creating {
            self.creating = false;
            cx.notify();
        }
    }

    pub(super) fn submit_create(&mut self, title: String, detail: String, cx: &mut Context<Self>) {
        let database = self.database.clone();
        let project = self.project.clone();
        let session_id = self.active_session.as_ref().map(|(id, _)| id.clone());
        let operation = match &self.state {
            PlanLoadState::Ready(data) => data.snapshot.as_ref().map(|snapshot| {
                let plan = snapshot.plan.number;
                let after = self
                    .selected
                    .or_else(|| snapshot.walk.as_ref().and_then(|walk| walk.current_node));
                let files = detail
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                (plan, after, files)
            }),
            PlanLoadState::Loading | PlanLoadState::Failed(_) => return,
        };
        let edit = cx.background_spawn(async move {
            if let Some((plan, after, files)) = operation {
                add_node(database, project, plan, title, files, after, session_id)
            } else {
                let root = if detail.trim().is_empty() {
                    "Current state".to_owned()
                } else {
                    detail
                };
                create_plan(database, project, title, root)
            }
        });
        self.state = PlanLoadState::Loading;
        self.refresh = Some(cx.spawn(async move |weak, cx| {
            let result = edit.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok((data, number)) => {
                        this.state = PlanLoadState::Ready(Box::new(data));
                        this.selected = Some(number);
                        this.creating = false;
                    }
                    Err(error) => this.state = PlanLoadState::Failed(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn link_active_session(&mut self, walk: u64, cx: &mut Context<Self>) {
        let Some((session_id, session_path)) = self.active_session.clone() else {
            return;
        };
        let database = self.database.clone();
        let project = self.project.clone();
        let edit = cx.background_spawn(async move {
            link_session(database, project, walk, session_id, session_path)
        });
        self.state = PlanLoadState::Loading;
        self.refresh = Some(cx.spawn(async move |weak, cx| {
            let state = match edit.await {
                Ok(data) => PlanLoadState::Ready(Box::new(data)),
                Err(error) => PlanLoadState::Failed(error),
            };
            let _ = weak.update(cx, |this, cx| {
                this.state = state;
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Render for WorkGraphBoardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.search.is_none() {
            let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search plan"));
            self.subscriptions.push(cx.subscribe_in(
                &search,
                window,
                |_, _, _: &InputEvent, _, cx| cx.notify(),
            ));
            self.search = Some(search);
        }
        if self.create_title.is_none() {
            self.create_title =
                Some(cx.new(|cx| InputState::new(window, cx).placeholder("Outcome title")));
        }
        if self.create_detail.is_none() {
            self.create_detail = Some(cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(3, 8)
                    .submit_on_enter(false)
                    .placeholder("Current state, or one scoped path per line")
            }));
        }
        let entity = cx.entity();
        let viewport_width = window.viewport_size().width;
        let shell_layout = crate::layout::layout_mode(viewport_width);
        let board_width = if crate::layout::shows_left_inline(shell_layout) {
            viewport_width - THEME.layout.session_rail
        } else {
            viewport_width
        };
        let layout = board_layout_mode(board_width);
        div()
            .size_full()
            .track_focus(&self.focus)
            .key_context(WORKGRAPH_KEY_CONTEXT)
            .min_h_0()
            .bg(THEME.colors.panel)
            .child(match &self.state {
                PlanLoadState::Loading => feedback(
                    "workgraph-loading",
                    "Loading plan…",
                    FeedbackTone::Info,
                )
                .into_any_element(),
                PlanLoadState::Failed(error) => {
                    let retry = entity.clone();
                    div()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.sm)
                        .child(feedback(
                            "workgraph-error",
                            error.clone(),
                            FeedbackTone::Error,
                        ))
                        .child(button(
                            "workgraph-retry",
                            "Try again",
                            ButtonTone::Neutral,
                            true,
                            move |_, cx| retry.update(cx, |this, cx| this.refresh(cx)),
                        ))
                        .into_any_element()
                }
                PlanLoadState::Ready(data) => {
                    let search = self
                        .search
                        .as_ref()
                        .map(|input| input.read(cx).value().to_string())
                        .unwrap_or_default();
                    let rows = data
                        .snapshot
                        .as_ref()
                        .map(|snapshot| plan_rows(snapshot, &search))
                        .unwrap_or_default();
                    let reached = rows.iter().filter(|row| row.reached).count();
                    let total = data
                        .snapshot
                        .as_ref()
                        .map_or(0, |snapshot| snapshot.nodes.len());
                    let title = data
                        .snapshot
                        .as_ref()
                        .map_or("Plans", |snapshot| snapshot.plan.title.as_str());
                    let refresh = entity.clone();
                    let create = entity.clone();
                    div()
                        .size_full()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .h(px(58.0))
                                .flex_none()
                                .px(THEME.space.md)
                                .flex()
                                .items_center()
                                .justify_between()
                                .border_b(THEME.border)
                                .border_color(THEME.colors.border)
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap(px(2.0))
                                        .child(
                                            div()
                                                .text_size(THEME.type_scale.body)
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(title.to_owned()),
                                        )
                                        .child(
                                            div()
                                                .text_size(THEME.type_scale.caption)
                                                .text_color(THEME.colors.subtle)
                                                .child(if total == 0 {
                                                    "Create a plan to establish the current state."
                                                        .to_owned()
                                                } else {
                                                    format!(
                                                        "{reached} of {total} states reached · {} plan(s)",
                                                        data.plans.len()
                                                    )
                                                }),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(THEME.space.xs)
                                        .child(
                                            Input::new(
                                                self.search
                                                    .as_ref()
                                                    .expect("plan search initialized"),
                                            )
                                            .w(px(210.0)),
                                        )
                                        .child(button(
                                            "workgraph-refresh",
                                            "Refresh",
                                            ButtonTone::Quiet,
                                            true,
                                            move |_, cx| {
                                                refresh.update(cx, |this, cx| this.refresh(cx));
                                            },
                                        ))
                                        .child(button(
                                            "workgraph-create-open",
                                            if data.snapshot.is_some() {
                                                "Add node"
                                            } else {
                                                "New plan"
                                            },
                                            ButtonTone::Neutral,
                                            true,
                                            move |_, cx| {
                                                create.update(cx, |this, cx| this.start_create(cx));
                                            },
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .flex()
                                .when(
                                    !self.creating
                                        && (layout != BoardLayoutMode::Narrow
                                            || self.selected.is_none()),
                                    |board| {
                                        board.child(if data.snapshot.is_none() {
                                            div()
                                                .id("workgraph-empty")
                                                .flex_1()
                                                .h_full()
                                                .flex()
                                                .flex_col()
                                                .items_center()
                                                .justify_center()
                                                .gap(THEME.space.xs)
                                                .child(
                                                    div()
                                                        .text_size(THEME.type_scale.body)
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .child("No plan yet"),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(THEME.type_scale.caption)
                                                        .text_color(THEME.colors.subtle)
                                                        .child("Start with the product as it is now."),
                                                )
                                                .into_any_element()
                                        } else {
                                            render_plan_list(
                                                rows,
                                                self.selected,
                                                entity.clone(),
                                            )
                                            .into_any_element()
                                        })
                                    },
                                )
                                .when(self.creating, |board| {
                                    board.child(render_create(
                                        self.create_title
                                            .as_ref()
                                            .expect("create title initialized"),
                                        self.create_detail
                                            .as_ref()
                                            .expect("create detail initialized"),
                                        data.snapshot.is_some(),
                                        entity.clone(),
                                    ))
                                })
                                .when(
                                    !self.creating
                                        && data.snapshot.is_some()
                                        && (layout != BoardLayoutMode::Narrow
                                            || self.selected.is_some()),
                                    |board| {
                                        board.child(self.render_detail(
                                            entity,
                                            data,
                                            layout,
                                            false,
                                        ))
                                    },
                                ),
                        )
                        .into_any_element()
                }
            })
    }
}
