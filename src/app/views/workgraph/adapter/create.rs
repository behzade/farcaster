use gpui::{AppContext as _, Context, Focusable as _, Window};

use super::WorkGraphBoardView;
use crate::app::views::workgraph::{contract::PlanLoadState, core::create_form_valid};
use workgraph::{add_node, create_plan};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app::views::workgraph) enum CreateStage {
    Closed,
    Node,
    CurrentState,
    Outcome,
}

impl CreateStage {
    pub(in crate::app::views::workgraph) const fn is_open(self) -> bool {
        !matches!(self, Self::Closed)
    }
}

impl WorkGraphBoardView {
    pub(crate) fn start_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.create_stage.is_open() {
            return;
        }
        if let Some(title) = &self.create_title {
            title.update(cx, |input, cx| input.set_value(String::new(), window, cx));
        }
        if let Some(detail) = &self.create_detail {
            detail.update(cx, |input, cx| input.set_value(String::new(), window, cx));
        }
        self.create_stage = if matches!(&self.state, PlanLoadState::Ready(data) if data.snapshot.is_some())
        {
            CreateStage::Node
        } else {
            CreateStage::CurrentState
        };
        self.pending_create_focus = true;
        cx.notify();
    }

    pub(in crate::app::views::workgraph) fn next_create_step(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let has_current_state = self
            .create_detail
            .as_ref()
            .is_some_and(|detail| !detail.read(cx).value().trim().is_empty());
        if self.create_stage == CreateStage::CurrentState && has_current_state {
            self.create_stage = CreateStage::Outcome;
            if let Some(title) = &self.create_title {
                title.read(cx).focus_handle(cx).focus(window, cx);
            }
            cx.notify();
        }
    }

    pub(in crate::app::views::workgraph) fn previous_create_step(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.create_stage == CreateStage::Outcome {
            self.create_stage = CreateStage::CurrentState;
            if let Some(detail) = &self.create_detail {
                detail.read(cx).focus_handle(cx).focus(window, cx);
            }
            cx.notify();
        }
    }

    pub(in crate::app::views::workgraph) fn cancel_create(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.create_stage.is_open() {
            self.create_stage = CreateStage::Closed;
            self.pending_create_focus = false;
            self.focus.focus(window, cx);
            cx.notify();
        }
    }

    pub(in crate::app::views::workgraph) fn submit_create_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(title_input) = &self.create_title else {
            return;
        };
        let Some(detail_input) = &self.create_detail else {
            return;
        };
        let title = title_input.read(cx).value().trim().to_owned();
        let detail = detail_input.read(cx).value().trim().to_owned();
        let has_plan = matches!(&self.state, PlanLoadState::Ready(data) if data.snapshot.is_some());
        if !create_form_valid(has_plan, &title, &detail) {
            return;
        }
        title_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
        detail_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
        self.focus.focus(window, cx);
        self.submit_create(title, detail, cx);
    }

    fn submit_create(&mut self, title: String, detail: String, cx: &mut Context<Self>) {
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
                create_plan(database, project, title, detail)
            }
        });
        self.state = PlanLoadState::Loading;
        self.refresh = Some(cx.spawn(async move |weak, cx| {
            let result = edit.await;
            let _ = weak.update(cx, |this, cx| {
                this.create_stage = CreateStage::Closed;
                this.pending_create_focus = false;
                match result {
                    Ok((data, number)) => {
                        this.state = PlanLoadState::Ready(Box::new(data));
                        this.selected = Some(number);
                    }
                    Err(error) => this.state = PlanLoadState::Failed(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
