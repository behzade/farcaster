//! GPUI and SQLite adapters for the workgraph plan list.

mod create;
mod detail;

pub(super) use create::CreateStage;

use std::path::PathBuf;

use super::{
    components::{render_create_step, render_plan_list},
    contract::PlanLoadState,
    core::{adjacent_node_number, create_form_valid, plan_rows},
    layout::BoardLayoutMode,
    persistence::{link_session, load_plan},
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
    create_stage: CreateStage,
    pub(super) active_session: Option<(String, String)>,
    search: Option<Entity<InputState>>,
    create_title: Option<Entity<InputState>>,
    create_detail: Option<Entity<TextareaState>>,
    pending_create_focus: bool,
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
            create_stage: CreateStage::Closed,
            active_session: None,
            search: None,
            create_title: None,
            create_detail: None,
            pending_create_focus: false,
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
                    && let Some(selected) = this.selected
                    && !data.snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot.nodes.iter().any(|node| node.number == selected)
                    })
                {
                    this.selected = None;
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(crate) fn select_node(&mut self, number: u64, cx: &mut Context<Self>) {
        if self.selected != Some(number) || self.create_stage.is_open() {
            self.selected = Some(number);
            self.create_stage = CreateStage::Closed;
            cx.notify();
        }
    }

    pub(super) fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.selected.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub(crate) fn prepare_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = None;
        self.create_stage = CreateStage::Closed;
        self.pending_create_focus = false;
        if let Some(search) = &self.search {
            search.update(cx, |input, cx| {
                input.set_value(String::new(), window, cx);
            });
        }
        self.focus.focus(window, cx);
        cx.notify();
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

    pub(crate) fn dismiss_work_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.create_stage {
            CreateStage::Outcome => {
                self.previous_create_step(window, cx);
                return true;
            }
            CreateStage::Node | CreateStage::CurrentState => {
                self.cancel_create(window, cx);
                return true;
            }
            CreateStage::Closed => {}
        }
        if let Some(search) = &self.search
            && !search.read(cx).value().is_empty()
        {
            search.update(cx, |input, cx| {
                input.set_value(String::new(), window, cx);
            });
            return true;
        }
        if self.selected.take().is_some() {
            cx.notify();
            return true;
        }
        false
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
            let title =
                cx.new(|cx| InputState::new(window, cx).placeholder("What should be true?"));
            self.subscriptions.push(cx.subscribe_in(
                &title,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { shift: false, .. } => {
                        this.submit_create_inputs(window, cx);
                    }
                    InputEvent::PressEnter { .. } | InputEvent::Blur | InputEvent::Focus => {}
                },
            ));
            self.create_title = Some(title);
        }
        if self.create_detail.is_none() {
            let detail = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(3, 6)
                    .submit_on_enter(false)
            });
            self.subscriptions.push(cx.subscribe_in(
                &detail,
                window,
                |_, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                },
            ));
            self.create_detail = Some(detail);
        }
        if self.pending_create_focus {
            match self.create_stage {
                CreateStage::Node | CreateStage::Outcome => {
                    if let Some(title) = &self.create_title {
                        title.read(cx).focus_handle(cx).focus(window, cx);
                    }
                }
                CreateStage::CurrentState => {
                    if let Some(detail) = &self.create_detail {
                        detail.read(cx).focus_handle(cx).focus(window, cx);
                    }
                }
                CreateStage::Closed => {}
            }
            self.pending_create_focus = false;
        }
        let entity = cx.entity();
        div()
            .size_full()
            .track_focus(&self.focus)
            .key_context(WORKGRAPH_KEY_CONTEXT)
            .min_h_0()
            .bg(THEME.colors.panel)
            .child(match &self.state {
                PlanLoadState::Loading => {
                    feedback("workgraph-loading", "Loading plan…", FeedbackTone::Info)
                        .into_any_element()
                }
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
                    let has_plan = data.snapshot.is_some();
                    let title = self
                        .create_title
                        .as_ref()
                        .expect("create title initialized");
                    let detail = self
                        .create_detail
                        .as_ref()
                        .expect("create detail initialized");
                    let current_state_complete = !detail.read(cx).value().trim().is_empty();
                    let can_submit = create_form_valid(
                        has_plan,
                        title.read(cx).value().as_ref(),
                        detail.read(cx).value().as_ref(),
                    );

                    if self.create_stage.is_open() {
                        render_create_step(
                            title,
                            detail,
                            self.create_stage,
                            current_state_complete,
                            can_submit,
                            entity,
                        )
                        .into_any_element()
                    } else {
                        let plan_title = data
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
                                            .min_w_0()
                                            .text_size(THEME.type_scale.body)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(plan_title.to_owned()),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(THEME.space.xs)
                                            .when(has_plan && self.selected.is_none(), |actions| {
                                                actions.child(
                                                    Input::new(
                                                        self.search
                                                            .as_ref()
                                                            .expect("plan search initialized"),
                                                    )
                                                    .w(px(190.0)),
                                                )
                                            })
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
                                                if has_plan { "Add node" } else { "New plan" },
                                                ButtonTone::Neutral,
                                                true,
                                                move |window, cx| {
                                                    create.update(cx, |this, cx| {
                                                        this.start_create(window, cx);
                                                    });
                                                },
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .when_some(self.selected, |body, _| {
                                        body.child(self.render_detail(
                                            entity.clone(),
                                            data,
                                            BoardLayoutMode::Narrow,
                                            false,
                                        ))
                                    })
                                    .when(self.selected.is_none(), |body| {
                                        body.child(if has_plan {
                                            render_plan_list(rows, None, entity.clone())
                                                .into_any_element()
                                        } else {
                                            div()
                                                .id("workgraph-empty")
                                                .size_full()
                                                .flex()
                                                .flex_col()
                                                .items_center()
                                                .justify_center()
                                                .gap(THEME.space.sm)
                                                .child(
                                                    div()
                                                        .text_size(THEME.type_scale.body)
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .child("No plan yet"),
                                                )
                                                .child(button(
                                                    "workgraph-empty-create",
                                                    "New plan",
                                                    ButtonTone::Accent,
                                                    true,
                                                    move |window, cx| {
                                                        entity.update(cx, |this, cx| {
                                                            this.start_create(window, cx);
                                                        });
                                                    },
                                                ))
                                                .into_any_element()
                                        })
                                    }),
                            )
                            .into_any_element()
                    }
                }
            })
    }
}
