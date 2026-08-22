use std::path::PathBuf;

use gpui::{
    AppContext as _, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Task, WeakEntity, div,
    prelude::FluentBuilder as _,
};

use super::{
    components::{detail_card, detail_label},
    contract::{PlanData, PlanLoadState},
    core::active_steps,
    persistence::load_plan,
};
use crate::{
    app::PiApp,
    assets::AppIcon,
    primitives::{ButtonTone, FeedbackTone, button, feedback, icon_button, section_heading},
    theme::THEME,
};

pub(crate) struct WorkGraphSidebarView {
    app: WeakEntity<PiApp>,
    database: PathBuf,
    project: PathBuf,
    session_id: Option<String>,
    state: PlanLoadState,
    refresh: Option<Task<()>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SidebarSummary {
    plan_title: Option<String>,
    node: Option<workgraph::contract::Node>,
    complete: bool,
    attached: bool,
}

impl WorkGraphSidebarView {
    pub(crate) fn new(
        app: WeakEntity<PiApp>,
        database: Result<PathBuf, String>,
        project: PathBuf,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let (database, state) = match database {
            Ok(database) => (database, PlanLoadState::Loading),
            Err(error) => (PathBuf::new(), PlanLoadState::Failed(error)),
        };
        let should_refresh = matches!(state, PlanLoadState::Loading);
        let mut view = Self {
            app,
            database,
            project,
            session_id: None,
            state,
            refresh: None,
        };
        if should_refresh {
            view.refresh(cx);
        }
        view
    }

    pub(crate) fn refresh_for(
        &mut self,
        project: PathBuf,
        session_id: Option<String>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.project = project;
        self.session_id = session_id;
        self.refresh(cx);
    }

    pub(crate) fn refresh(&mut self, cx: &mut gpui::Context<Self>) {
        let notify_loading = prepare_refresh(&mut self.state);
        let database = self.database.clone();
        let project = self.project.clone();
        let session_id = self.session_id.clone();
        let load =
            cx.background_spawn(async move { load_plan(database, project, session_id.as_deref()) });
        self.refresh = Some(cx.spawn(async move |weak, cx| {
            let state = match load.await {
                Ok(data) => PlanLoadState::Ready(Box::new(data)),
                Err(error) => PlanLoadState::Failed(error),
            };
            let _ = weak.update(cx, |this, cx| {
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
        let _timing = crate::performance::Timing::new("render.workgraph_sidebar");
        let entity = cx.entity().downgrade();
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(section_heading("Current plan"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(THEME.space.xs)
                    .child(icon_button(
                        "refresh-workgraph-sidebar",
                        AppIcon::ArrowsClockwise,
                        "Refresh plan",
                        ButtonTone::Quiet,
                        {
                            let entity = cx.entity().downgrade();
                            move |_, cx| {
                                if let Some(entity) = entity.upgrade() {
                                    entity.update(cx, |this, cx| this.refresh(cx));
                                }
                            }
                        },
                    ))
                    .child(icon_button(
                        "open-workgraph-from-sidebar",
                        AppIcon::ArrowsOut,
                        "View project plan",
                        ButtonTone::Quiet,
                        {
                            let app = self.app.clone();
                            move |window, cx| {
                                let _ = app.update(cx, |app, cx| {
                                    app.open_workgraph_surface(window, cx);
                                });
                            }
                        },
                    )),
            );
        div()
            .flex()
            .flex_col()
            .gap(THEME.space.xs)
            .child(header)
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
                PlanLoadState::Ready(data) => {
                    render_summary(sidebar_summary(data), self.app.clone()).into_any_element()
                }
            })
    }
}

fn sidebar_summary(data: &PlanData) -> SidebarSummary {
    let Some(snapshot) = &data.snapshot else {
        return SidebarSummary {
            plan_title: None,
            node: None,
            complete: false,
            attached: false,
        };
    };
    let complete = snapshot
        .walk
        .as_ref()
        .is_some_and(|walk| walk.current_node.is_none());
    let number = snapshot
        .walk
        .as_ref()
        .and_then(|walk| walk.current_node)
        .or_else(|| active_steps(snapshot).last().map(|step| step.node_number));
    SidebarSummary {
        plan_title: Some(snapshot.plan.title.clone()),
        node: number.and_then(|number| {
            snapshot
                .nodes
                .iter()
                .find(|node| node.number == number)
                .cloned()
        }),
        complete,
        attached: data.session_link.is_some(),
    }
}

fn render_summary(summary: SidebarSummary, app: WeakEntity<PiApp>) -> impl IntoElement {
    let SidebarSummary {
        plan_title,
        node,
        complete,
        attached,
    } = summary;
    let has_plan = plan_title.is_some();
    let open_plan = app.clone();
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .when_some(node, |card, node| {
            let number = node.number;
            card.child(
                div()
                    .id(("workgraph-sidebar-node", number))
                    .p(THEME.space.sm)
                    .flex()
                    .flex_col()
                    .gap(THEME.space.xs)
                    .rounded(THEME.radius)
                    .border(THEME.border)
                    .border_color(THEME.colors.border)
                    .bg(THEME.colors.canvas)
                    .cursor_pointer()
                    .hover(|card| card.bg(THEME.colors.surface))
                    .on_click(move |_, window, cx| {
                        let _ = app.update(cx, |app, cx| {
                            app.inspect_workgraph_issue(number, window, cx);
                        });
                    })
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if complete {
                                THEME.colors.success
                            } else {
                                THEME.colors.accent
                            })
                            .child(if complete {
                                "Outcome reached"
                            } else {
                                "Current node"
                            }),
                    )
                    .child(
                        div()
                            .line_clamp(2)
                            .text_size(THEME.type_scale.body_small)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(node.title),
                    )
                    .when_some(plan_title, |card, title| {
                        card.child(
                            div()
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.subtle)
                                .child(title),
                        )
                    })
                    .when(!attached, |card| {
                        card.child(
                            div()
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.warning)
                                .child("Session not attached"),
                        )
                    }),
            )
        })
        .when(!has_plan, |card| {
            card.child(
                detail_card()
                    .child(detail_label("No plan"))
                    .child(
                        div()
                            .text_size(THEME.type_scale.body_small)
                            .text_color(THEME.colors.muted)
                            .child("Create a plan from the product's current state."),
                    )
                    .child(button(
                        "open-empty-plan",
                        "Create plan",
                        ButtonTone::Quiet,
                        true,
                        move |window, cx| {
                            let _ = open_plan.update(cx, |app, cx| {
                                app.open_workgraph_surface(window, cx);
                            });
                        },
                    )),
            )
        })
}

#[cfg(test)]
mod tests {
    use workgraph::contract::{CompletionRequirement, Node, Plan, PlanSnapshot, SessionLink, Walk};

    use super::*;

    fn data(attached: bool) -> PlanData {
        let link = SessionLink {
            session_id: "session-1".into(),
            session_path: "/sessions/1.jsonl".into(),
            plan_number: 1,
            walk_number: 1,
            linked_at: 0,
        };
        let plan = Plan {
            project: "/project".into(),
            number: 1,
            title: "VCS integration".into(),
            root_node: 1,
            version: 1,
            created_at: 0,
            updated_at: 0,
        };
        PlanData {
            plans: vec![plan.clone()],
            snapshot: Some(PlanSnapshot {
                plan,
                nodes: vec![Node {
                    plan_number: 1,
                    number: 1,
                    title: "Current product".into(),
                    files: Vec::new(),
                    completion: CompletionRequirement::Observation,
                    version: 1,
                    created_at: 0,
                    updated_at: 0,
                }],
                edges: Vec::new(),
                walk: Some(Walk {
                    plan_number: 1,
                    number: 1,
                    current_node: Some(1),
                    head_step: None,
                    version: 1,
                    created_at: 0,
                    updated_at: 0,
                }),
                steps: Vec::new(),
                sessions: attached.then(|| link.clone()).into_iter().collect(),
            }),
            session_link: attached.then_some(link),
        }
    }

    #[test]
    fn ready_content_remains_visible_while_refreshing() {
        let data = data(false);
        let mut state = PlanLoadState::Ready(Box::new(data.clone()));
        assert!(!prepare_refresh(&mut state));
        assert_eq!(state, PlanLoadState::Ready(Box::new(data)));
    }

    #[test]
    fn summary_uses_the_linked_walk_current_node() {
        let summary = sidebar_summary(&data(true));
        assert_eq!(summary.node.as_ref().map(|node| node.number), Some(1));
        assert_eq!(summary.plan_title.as_deref(), Some("VCS integration"));
        assert!(summary.attached);
        assert!(!summary.complete);
    }
}
