mod create;
mod detail;

pub(super) use create::CreateStage;

use std::path::PathBuf;

use super::{
    components::{render_create_form, render_plan_list, render_session_goal},
    contract::{PlanData, PlanLoadState},
    core::{adjacent_node_number, create_form_valid, plan_rows},
    layout::{BoardLayoutMode, board_layout},
};
use crate::{
    app::ui::assets::AppIcon,
    app::ui::primitives::{ButtonTone, FeedbackTone, button, feedback, icon_button},
    app::ui::theme::THEME,
};
use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    input::{Input, InputEvent, InputState, TextareaState},
    kbd::Kbd,
};
use workgraph::{link_session, load_selected_plan};

pub(crate) const WORKGRAPH_KEY_CONTEXT: &str = "PiWorkGraph";
pub(crate) const WORKGRAPH_NAV_KEY_CONTEXT: &str = "PiWorkGraph && !Input";

pub(crate) struct WorkGraphBoardView {
    database: PathBuf,
    project: PathBuf,
    pub(super) state: PlanLoadState,
    focus: FocusHandle,
    pub(super) selected: Option<u64>,
    plan: Option<u64>,
    all_plans: bool,
    create_stage: CreateStage,
    pub(super) active_session: Option<(String, String)>,
    session_goal: Option<crate::agents::SessionGoal>,
    search: Entity<InputState>,
    create_title: Entity<InputState>,
    create_detail: Entity<TextareaState>,
    refresh: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl WorkGraphBoardView {
    pub(crate) fn new(
        database: Result<PathBuf, String>,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (database, state) = match database {
            Ok(database) => (database, PlanLoadState::Loading),
            Err(error) => (PathBuf::new(), PlanLoadState::Failed(error)),
        };
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let create_title =
            cx.new(|cx| InputState::new(window, cx).placeholder("What should be true?"));
        let create_detail = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 6)
                .submit_on_enter(false)
        });
        let subscriptions = vec![
            cx.subscribe_in(&search, window, |_, _, _: &InputEvent, _, cx| {
                cx.notify();
            }),
            cx.subscribe_in(
                &create_title,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { shift: false, .. } => {
                        this.submit_create_inputs(window, cx);
                    }
                    InputEvent::PressEnter { .. } | InputEvent::Blur | InputEvent::Focus => {}
                },
            ),
            cx.subscribe_in(&create_detail, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        let should_refresh = matches!(state, PlanLoadState::Loading);
        let mut view = Self {
            database,
            project,
            state,
            focus: cx.focus_handle(),
            selected: None,
            plan: None,
            all_plans: false,
            create_stage: CreateStage::Closed,
            active_session: None,
            session_goal: None,
            search,
            create_title,
            create_detail,
            refresh: None,
            _subscriptions: subscriptions,
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
        session_goal: Option<crate::agents::SessionGoal>,
        cx: &mut Context<Self>,
    ) {
        if self.project != project || self.active_session != active_session {
            self.plan = None;
            self.all_plans = false;
            self.selected = None;
            self.state = PlanLoadState::Loading;
        }
        self.project = project;
        self.active_session = active_session;
        self.session_goal = session_goal;
        self.refresh(cx);
    }

    pub(crate) fn set_session_goal(
        &mut self,
        goal: Option<crate::agents::SessionGoal>,
        cx: &mut Context<Self>,
    ) {
        if self.session_goal != goal {
            self.session_goal = goal;
            cx.notify();
        }
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        let database = self.database.clone();
        let project = self.project.clone();
        let session_id = self.active_session.as_ref().map(|(id, _)| id.clone());
        let plan = self.plan;
        let load = cx.background_spawn(async move {
            load_selected_plan(database, project, session_id.as_deref(), plan)
        });
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
        self.plan = None;
        self.all_plans = false;
        self.refresh(cx);
        self.reset_navigation(window, cx);
    }

    fn reset_navigation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = None;
        self.create_stage = CreateStage::Closed;
        self.search.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn back_to_plans(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(&self.state, PlanLoadState::Ready(data) if self.showing_plans(data)) {
            return;
        }
        self.all_plans = true;
        self.plan = None;
        self.reset_navigation(window, cx);
    }

    fn showing_plans(&self, data: &PlanData) -> bool {
        self.all_plans || (self.plan.is_none() && data.session_link.is_none())
    }

    fn open_plan(&mut self, plan: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.plan = Some(plan);
        self.all_plans = false;
        self.reset_navigation(window, cx);
        self.state = PlanLoadState::Loading;
        self.refresh(cx);
    }

    pub(crate) fn focus_search(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.read(cx).focus_handle(cx).focus(window, cx);
    }

    pub(crate) fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let PlanLoadState::Ready(data) = &self.state else {
            return;
        };
        if self.showing_plans(data) {
            return;
        }
        let Some(snapshot) = &data.snapshot else {
            return;
        };
        let search = self.search.read(cx).value().to_string();
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
        if self.create_stage.is_open() {
            self.cancel_create(window, cx);
            return true;
        }
        if !self.search.read(cx).value().is_empty() {
            self.search.update(cx, |input, cx| {
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

impl WorkGraphBoardView {
    fn render_state(&self, layout: BoardLayoutMode, cx: &mut Context<Self>) -> gpui::AnyElement {
        let entity = cx.entity();
        let notice = match &self.state {
            PlanLoadState::Ready(data) => return self.render_ready(data, entity, layout, cx),
            PlanLoadState::Loading => {
                feedback("workgraph-loading", "Loading plan…", FeedbackTone::Info)
                    .into_any_element()
            }
            PlanLoadState::Failed(error) => render_load_error(error, entity),
        };
        div()
            .p(THEME.space.md)
            .pr(px(56.0))
            .child(notice)
            .into_any_element()
    }

    fn render_ready(
        &self,
        data: &PlanData,
        entity: Entity<Self>,
        layout: BoardLayoutMode,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let has_plan = !self.showing_plans(data) && data.snapshot.is_some();
        if self.create_stage.is_open() {
            let can_submit = create_form_valid(
                has_plan,
                self.create_title.read(cx).value().as_ref(),
                self.create_detail.read(cx).value().as_ref(),
            );
            return render_create_form(
                &self.create_title,
                &self.create_detail,
                self.create_stage,
                can_submit,
                entity,
            )
            .into_any_element();
        }

        let search = self.search.read(cx).value().to_string();
        if self.showing_plans(data) {
            let query = search.trim().to_lowercase();
            let plans = data
                .plans
                .iter()
                .filter(|plan| plan.title.to_lowercase().contains(&query))
                .collect::<Vec<_>>();
            return div()
                .size_full()
                .flex()
                .flex_col()
                .child(render_board_header(
                    "All plans",
                    false,
                    true,
                    &self.search,
                    entity.clone(),
                ))
                .child(
                    div()
                        .id("workgraph-all-plans")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .p(THEME.space.md)
                        .gap(THEME.space.sm)
                        .when(data.plans.is_empty(), |list| {
                            list.child(render_empty_plan(entity.clone()))
                        })
                        .when(!data.plans.is_empty() && plans.is_empty(), |list| {
                            list.child("No plans match your search.")
                        })
                        .children(plans.into_iter().map(|plan| {
                            let entity = entity.clone();
                            let number = plan.number;
                            button(
                                format!("workgraph-plan-{number}"),
                                plan.title.clone(),
                                ButtonTone::Quiet,
                                true,
                                move |window, cx| {
                                    entity.update(cx, |this, cx| this.open_plan(number, window, cx))
                                },
                            )
                        })),
                )
                .into_any_element();
        }
        let rows = data
            .snapshot
            .as_ref()
            .map(|snapshot| plan_rows(snapshot, &search))
            .unwrap_or_default();
        let plan_title = data
            .snapshot
            .as_ref()
            .map_or("Project plan", |snapshot| snapshot.plan.title.as_str());
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(render_board_header(
                plan_title,
                has_plan,
                layout == BoardLayoutMode::Wide || self.selected.is_none(),
                &self.search,
                entity.clone(),
            ))
            .when_some(self.session_goal.as_ref(), |board, goal| {
                board.child(render_session_goal(goal, false))
            })
            .child(self.render_board_body(data, rows, has_plan, entity, layout))
            .into_any_element()
    }

    fn render_board_body(
        &self,
        data: &PlanData,
        rows: Vec<super::contract::PlanRow>,
        has_plan: bool,
        entity: Entity<Self>,
        layout: BoardLayoutMode,
    ) -> impl IntoElement {
        let split = layout == BoardLayoutMode::Wide && has_plan;
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .when(split || self.selected.is_none(), |body| {
                body.child(if has_plan {
                    render_plan_list(rows, self.selected, entity.clone()).into_any_element()
                } else {
                    render_empty_plan(entity.clone()).into_any_element()
                })
            })
            .when(split || self.selected.is_some(), |body| {
                body.child(self.render_detail(entity, data, layout, false))
            })
    }
}

fn render_load_error(error: &str, entity: Entity<WorkGraphBoardView>) -> gpui::AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.sm)
        .child(feedback(
            "workgraph-error",
            error.to_owned(),
            FeedbackTone::Error,
        ))
        .child(button(
            "workgraph-retry",
            "Try again",
            ButtonTone::Neutral,
            true,
            move |_, cx| entity.update(cx, |this, cx| this.refresh(cx)),
        ))
        .into_any_element()
}

fn render_board_header(
    plan_title: &str,
    has_plan: bool,
    show_list: bool,
    search: &Entity<InputState>,
    entity: Entity<WorkGraphBoardView>,
) -> impl IntoElement {
    let refresh = entity.clone();
    let back = entity.clone();
    let plans = entity.clone();
    div()
        .h(px(56.0))
        .flex_none()
        .pl(THEME.space.md)
        .pr(px(56.0))
        .gap(THEME.space.sm)
        .flex()
        .items_center()
        .justify_between()
        .border_b(THEME.border)
        .border_color(THEME.colors.surface)
        .when(has_plan, |header| {
            header.child(
                button(
                    "workgraph-all-plans-back",
                    "Back",
                    ButtonTone::Quiet,
                    true,
                    move |window, cx| plans.update(cx, |this, cx| this.back_to_plans(window, cx)),
                )
                .tooltip("Back to all plans")
                .child(
                    Kbd::new(gpui::Keystroke::parse("backspace").expect("static shortcut"))
                        .outline(),
                ),
            )
        })
        .child(if show_list {
            div()
                .min_w_0()
                .flex_1()
                .truncate()
                .text_size(THEME.type_scale.reading)
                .font_weight(FontWeight::SEMIBOLD)
                .child(plan_title.to_owned())
                .into_any_element()
        } else {
            button(
                "workgraph-detail-back",
                "← Back to plan",
                ButtonTone::Quiet,
                true,
                move |_, cx| back.update(cx, |this, cx| this.clear_selection(cx)),
            )
            .into_any_element()
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(THEME.space.xs)
                .when(show_list, |actions| {
                    actions.child(Input::new(search).w(px(140.0)))
                })
                .child(icon_button(
                    "workgraph-refresh",
                    AppIcon::ArrowsClockwise,
                    "Refresh plan",
                    ButtonTone::Quiet,
                    move |_, cx| refresh.update(cx, |this, cx| this.refresh(cx)),
                ))
                .when(show_list, |actions| {
                    actions.child(button(
                        "workgraph-create-open",
                        if has_plan { "Add node" } else { "New plan" },
                        ButtonTone::Neutral,
                        true,
                        move |window, cx| {
                            entity.update(cx, |this, cx| this.start_create(window, cx));
                        },
                    ))
                }),
        )
}

fn render_empty_plan(entity: Entity<WorkGraphBoardView>) -> impl IntoElement {
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
                entity.update(cx, |this, cx| this.start_create(window, cx));
            },
        ))
}

impl Render for WorkGraphBoardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layout = board_layout(f32::from(window.viewport_size().width));
        div()
            .size_full()
            .track_focus(&self.focus)
            .key_context(WORKGRAPH_KEY_CONTEXT)
            .min_h_0()
            .bg(THEME.colors.panel)
            .child(self.render_state(layout, cx))
    }
}
