mod agents;
mod background_jobs;
pub(crate) mod change_tree;
mod performance;
mod repository;
mod repository_controls;
mod repository_presentation;
mod resize;
#[cfg(test)]
mod tests;

use gpui::{
    InteractiveElement as _, IntoElement, ParentElement as _, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};

pub(super) use resize::clamped_run_panel_width;

use self::{
    agents::{
        AgentSection, MAX_VISIBLE_COMPLETED_AGENTS, RunDisclosure, agent_section,
        disclosure_control,
    },
    background_jobs::background_job_row,
    performance::render_performance,
};
use super::super::{FarcasterApp, RunPanelView};
use crate::{
    agent_activity::AgentActivity,
    app::ui::primitives::{ButtonTone, button, panel, section_heading},
    app::ui::theme::THEME,
    sessions::{descendant_sessions, root_session_for_path},
};

/// Borrowed browser inputs shared by the panel and repository renderers.
pub(crate) struct RepositoryView<'a> {
    pub(crate) state: &'a change_tree::ChangeTreeState,
    pub(crate) search: &'a gpui::Entity<gpui_component::input::InputState>,
    pub(crate) query: &'a str,
    pub(crate) scroll: &'a ScrollHandle,
}

impl FarcasterApp {
    pub(super) fn render_run_panel(
        &self,
        entity: WeakEntity<Self>,
        run_panel: WeakEntity<RunPanelView>,
        completed_agents_expanded: bool,
        limited_agents_expanded: bool,
        browser: &RepositoryView<'_>,
    ) -> impl IntoElement {
        let root = root_session_for_path(
            &self.all_sessions,
            self.snapshot.selected_session.as_deref(),
        );
        let descendants = root
            .map(|root| descendant_sessions(&self.all_sessions, &root.id))
            .unwrap_or_default();
        let mut active = Vec::new();
        let mut completed = Vec::new();
        let mut limited = Vec::new();
        for (session, depth) in descendants {
            let Some(activity) = self.agent_activities.get(&session.id) else {
                continue;
            };
            match agent_section(activity.lifecycle, activity.limited, session.is_running) {
                AgentSection::Active => active.push((activity, depth, session)),
                AgentSection::Completed => completed.push((activity, depth, session)),
                AgentSection::Limited => limited.push((activity, depth, session)),
                AgentSection::Hidden => {}
            }
        }
        let by_created_at =
            |left: &(&AgentActivity, usize, &crate::sessions::SessionSummary),
             right: &(&AgentActivity, usize, &crate::sessions::SessionSummary)| {
                right
                    .2
                    .timestamp
                    .cmp(&left.2.timestamp)
                    .then_with(|| left.2.id.cmp(&right.2.id))
            };
        active.sort_by(by_created_at);
        completed.sort_by(by_created_at);
        limited.sort_by(by_created_at);

        let completed_control = disclosure_control(
            "toggle-completed-agents",
            "Completed agents",
            completed_agents_expanded,
            RunDisclosure::Completed,
            run_panel.clone(),
        );
        let limited_control = disclosure_control(
            "toggle-limited-agents",
            "Limited agents",
            limited_agents_expanded,
            RunDisclosure::Limited,
            run_panel.clone(),
        );
        let activity = div()
            .id("run-panel-activity")
            .flex_none()
            .min_h_0()
            .max_h(px(240.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(THEME.space.sm)
            .child(self.workgraph_sidebar_view.clone())
            .when_some(self.performance_monitor.as_ref(), |run, monitor| {
                run.child(render_performance(&monitor.summary))
            })
            .when(!active.is_empty(), |run| {
                run.child(
                    inspector_section()
                        .child(section_heading(format!(
                            "Workers · {} active",
                            active.len()
                        )))
                        .children(active.iter().filter_map(|(activity, depth, _)| {
                            self.agent_card(activity, *depth, false, entity.clone())
                        })),
                )
            })
            .when(!self.background_jobs.is_empty(), |run| {
                run.child(
                    inspector_section()
                        .child(section_heading(format!(
                            "Background jobs ({})",
                            self.background_jobs.len()
                        )))
                        .children(self.background_jobs.iter().map(background_job_row)),
                )
            })
            .when(!completed.is_empty(), |run| {
                run.child(
                    inspector_section()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(section_heading(format!(
                                    "Completed workers ({})",
                                    completed.len()
                                )))
                                .child(completed_control),
                        )
                        .when(completed_agents_expanded, |section| {
                            section
                                .children(
                                    completed
                                        .iter()
                                        .take(MAX_VISIBLE_COMPLETED_AGENTS)
                                        .filter_map(|(activity, depth, _)| {
                                            self.agent_card(activity, *depth, false, entity.clone())
                                        }),
                                )
                                .when(completed.len() > MAX_VISIBLE_COMPLETED_AGENTS, |section| {
                                    section.child(
                                        div()
                                            .text_size(THEME.type_scale.caption)
                                            .text_color(THEME.colors.subtle)
                                            .child(format!(
                                                "Showing the {} most recent completed agents",
                                                MAX_VISIBLE_COMPLETED_AGENTS
                                            )),
                                    )
                                })
                        }),
                )
            })
            .when(!limited.is_empty(), |run| {
                run.child(
                    inspector_section()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(section_heading(format!(
                                    "Limited workers ({})",
                                    limited.len()
                                )))
                                .child(limited_control),
                        )
                        .when(limited_agents_expanded, |section| {
                            section.children(limited.iter().filter_map(|(activity, depth, _)| {
                                self.agent_card(activity, *depth, true, entity.clone())
                            }))
                        }),
                )
            });
        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .pt(px(17.0))
            .pr(px(15.0))
            .pb(px(14.0))
            .pl(px(18.0))
            .gap(THEME.space.md)
            // Keep the root reachable independently of worker lifecycle and scroll position.
            .when_some(root, |run, root| {
                let selected =
                    self.snapshot.selected_session.as_deref() == Some(root.path.as_path());
                let path = root.path.clone();
                let project = root.project.clone();
                let entity = entity.clone();
                run.child(
                    button(
                        "run-panel-main-agent",
                        "Main agent",
                        if selected {
                            ButtonTone::Neutral
                        } else {
                            ButtonTone::Quiet
                        },
                        true,
                        move |window, cx| {
                            let _ = entity.update(cx, |this, cx| {
                                this.select_session(path.clone(), project.clone(), window, cx);
                            });
                        },
                    )
                    .w_full()
                    .flex_none(),
                )
            })
            .child(activity)
            .when(
                !self.repository.execution_allowed
                    || self.repository.backend.is_some()
                    || self.repository.error.is_some(),
                |run| run.child(self.render_repository(entity.clone(), run_panel.clone(), browser)),
            );
        panel()
            .size_full()
            .rounded_none()
            .border_0()
            .bg(THEME.colors.inspector)
            .child(body)
    }
}

fn inspector_section() -> gpui::Div {
    div().flex().flex_col().gap(px(7.0))
}
