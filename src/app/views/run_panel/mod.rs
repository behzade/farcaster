pub(in crate::app) mod agents;
mod background_jobs;
pub(crate) use crate::app::ui::change_tree;
mod performance;
mod repository;
mod repository_controls;
mod repository_presentation;
mod resize;
pub(super) mod review;
#[cfg(test)]
mod tests;

use gpui::{
    InteractiveElement as _, IntoElement, ParentElement as _, ScrollAnchor, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};

pub(super) use resize::clamped_run_panel_width;

use self::{
    agents::{AgentSection, agent_section, conversation_row},
    background_jobs::background_job_row,
    performance::render_performance,
};
use super::super::{FarcasterApp, RunPanelView};
use crate::{
    agent_activity::AgentActivity,
    app::ui::primitives::{ButtonTone, activates_button, button, panel, section_heading},
    app::ui::theme::theme,
    sessions::{descendant_sessions_for_root, root_session_for_path},
};

pub(crate) struct RepositoryView<'a> {
    pub(crate) state: &'a change_tree::ChangeTreeState,
    pub(crate) search: &'a gpui::Entity<gpui_component::input::InputState>,
    pub(crate) query: &'a str,
    pub(crate) scroll: &'a ScrollHandle,
}

pub(in crate::app) const RECENT_WORKERS: usize = 3;
pub(in crate::app) type WorkerProfileNames =
    std::collections::HashMap<(std::path::PathBuf, crate::agents::Backend, String), String>;

fn run_panel_agent_rows<'a>(
    sessions: &'a [crate::sessions::SessionSummary],
    activities: &std::collections::HashMap<String, AgentActivity>,
    selected: Option<&std::path::Path>,
) -> Vec<(
    AgentActivity,
    usize,
    &'a crate::sessions::SessionSummary,
    AgentSection,
)> {
    let Some(root) = root_session_for_path(sessions, selected) else {
        return Vec::new();
    };
    descendant_sessions_for_root(sessions, root)
        .into_iter()
        .filter_map(|(session, depth)| {
            let activity_key = crate::agent_activity::agent_activity_key(&session.path);
            let activity = activities
                .get(&activity_key)
                .cloned()
                .unwrap_or_else(|| AgentActivity::limited_fallback(session));
            let section = agent_section(activity.lifecycle, activity.limited, session.is_running);
            (section != AgentSection::Hidden).then_some((activity, depth, session, section))
        })
        .collect()
}

fn ordered_worker_rows<'a>(
    sessions: &'a [crate::sessions::SessionSummary],
    activities: &std::collections::HashMap<String, AgentActivity>,
    selected: Option<&std::path::Path>,
) -> Vec<(
    AgentActivity,
    usize,
    &'a crate::sessions::SessionSummary,
    AgentSection,
)> {
    let mut workers = run_panel_agent_rows(sessions, activities, selected);
    workers.sort_by(|left, right| {
        right
            .2
            .timestamp
            .cmp(&left.2.timestamp)
            .then_with(|| left.2.id.cmp(&right.2.id))
    });
    workers
}

pub(in crate::app) fn worker_navigation_rows<'a>(
    sessions: &'a [crate::sessions::SessionSummary],
    activities: &std::collections::HashMap<String, AgentActivity>,
    selected: Option<&std::path::Path>,
) -> Vec<&'a crate::sessions::SessionSummary> {
    let Some(root) = root_session_for_path(sessions, selected) else {
        return Vec::new();
    };
    std::iter::once(root)
        .chain(
            ordered_worker_rows(sessions, activities, selected)
                .into_iter()
                .map(|(_, _, session, _)| session),
        )
        .collect()
}

#[cfg(test)]
pub(crate) fn live_run_panel_agent_rows<'a>(
    sessions: &'a [crate::sessions::SessionSummary],
    activities: &std::collections::HashMap<String, AgentActivity>,
    selected: Option<&std::path::Path>,
) -> Vec<(
    AgentActivity,
    usize,
    &'a crate::sessions::SessionSummary,
    AgentSection,
)> {
    run_panel_agent_rows(sessions, activities, selected)
}

impl FarcasterApp {
    pub(super) fn render_older_workers_panel(
        &self,
        entity: WeakEntity<Self>,
        run_panel: WeakEntity<RunPanelView>,
        scroll: &ScrollHandle,
        anchor: &ScrollAnchor,
        saved_profiles: &WorkerProfileNames,
    ) -> impl IntoElement {
        let selected = self
            .lifecycle
            .pending_session_switch
            .as_ref()
            .map(|(path, _)| path.as_path())
            .or(self.snapshot.selected_session.as_deref());
        let workers = ordered_worker_rows(&self.sessions.all, &self.activity.agents, selected);
        let back_panel = run_panel.clone();
        panel()
            .size_full()
            .rounded_none()
            .border_0()
            .bg(theme().colors.inspector)
            .child(
                div()
                    .size_full()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_none()
                            .px(px(15.0))
                            .py(theme().space.sm)
                            .border_b(theme().border)
                            .border_color(theme().colors.border)
                            .flex()
                            .items_center()
                            .gap(theme().space.sm)
                            .child(button(
                                "close-older-workers",
                                "Back",
                                ButtonTone::Quiet,
                                true,
                                move |_, cx| {
                                    let _ = back_panel.update(cx, |view, cx| {
                                        view.close_older_workers();
                                        cx.notify();
                                    });
                                },
                            ))
                            .child(section_heading("Older workers")),
                    )
                    .child(
                        div()
                            .id("older-workers-list")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(scroll)
                            .px(px(15.0))
                            .py(theme().space.sm)
                            .children(workers.iter().skip(RECENT_WORKERS).filter_map(
                                |(activity, depth, session, _)| {
                                    self.agent_card(
                                        activity,
                                        session,
                                        *depth,
                                        selected == Some(session.path.as_path()),
                                        anchor,
                                        saved_profiles,
                                        entity.clone(),
                                    )
                                },
                            )),
                    ),
            )
    }

    pub(super) fn render_run_panel(
        &self,
        entity: WeakEntity<Self>,
        run_panel: WeakEntity<RunPanelView>,
        scroll: &ScrollHandle,
        anchor: &ScrollAnchor,
        saved_profiles: &WorkerProfileNames,
        browser: &RepositoryView<'_>,
    ) -> impl IntoElement {
        let selected = self
            .lifecycle
            .pending_session_switch
            .as_ref()
            .map(|(path, _)| path.as_path())
            .or(self.snapshot.selected_session.as_deref());
        let root = root_session_for_path(&self.sessions.all, selected);
        let workers = ordered_worker_rows(&self.sessions.all, &self.activity.agents, selected);
        let older_count = workers.len().saturating_sub(RECENT_WORKERS);
        let selected_is_older = workers
            .iter()
            .skip(RECENT_WORKERS)
            .any(|(_, _, session, _)| selected == Some(session.path.as_path()));
        let root_path = root.map(|session| session.path.clone());
        let older_panel = run_panel.clone();
        let conversation = inspector_section()
            .when_some(root, |section, root| {
                let is_selected = selected == Some(root.path.as_path());
                let path = root.path.clone();
                let project = root.project.clone();
                let app = entity.clone();
                let key_path = path.clone();
                let key_project = project.clone();
                let key_app = app.clone();
                section.child(
                    conversation_row("run-panel-main-agent", is_selected)
                        .anchor_scroll(is_selected.then(|| anchor.clone()))
                        .aria_label("Show main agent transcript")
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            crate::app::ui::primitives::preserve_pointer_focus,
                        )
                        .on_click(move |_, window, cx| {
                            let _ = app.update(cx, |this, cx| {
                                this.select_session_and_focus(
                                    path.clone(),
                                    project.clone(),
                                    window,
                                    cx,
                                );
                            });
                        })
                        .on_key_down(move |event, window, cx| {
                            if activates_button(event) {
                                cx.stop_propagation();
                                let _ = key_app.update(cx, |this, cx| {
                                    this.select_session_and_focus(
                                        key_path.clone(),
                                        key_project.clone(),
                                        window,
                                        cx,
                                    );
                                });
                            }
                        })
                        .child("Main agent"),
                )
            })
            .children(
                workers
                    .iter()
                    .enumerate()
                    .filter(|(index, (_, _, session, _))| {
                        *index < RECENT_WORKERS || selected == Some(session.path.as_path())
                    })
                    .filter_map(|(_, (activity, depth, session, _))| {
                        self.agent_card(
                            activity,
                            session,
                            *depth,
                            selected == Some(session.path.as_path()),
                            anchor,
                            saved_profiles,
                            entity.clone(),
                        )
                    }),
            )
            .when(older_count > 0, |section| {
                section.child(button(
                    "show-older-workers",
                    format!("Show older workers ({older_count})"),
                    ButtonTone::Quiet,
                    true,
                    move |_, cx| {
                        if let Some(root) = root_path.clone() {
                            let _ = older_panel.update(cx, |view, cx| {
                                view.show_older_workers(root, selected_is_older);
                                cx.notify();
                            });
                        }
                    },
                ))
            });
        let activity = div()
            .id("run-panel-activity")
            .flex_none()
            .min_h_0()
            .max_h(px(320.0))
            .overflow_y_scroll()
            .track_scroll(scroll)
            .flex()
            .flex_col()
            .gap(theme().space.sm)
            .child(conversation)
            .child(self.views.workgraph_sidebar.clone())
            .when_some(
                self.lifecycle
                    .performance_monitor
                    .as_ref()
                    .filter(|monitor| monitor.is_detailed()),
                |run, monitor| run.child(render_performance(&monitor.summary)),
            )
            .when(!self.activity.background_jobs.is_empty(), |run| {
                run.child(
                    inspector_section()
                        .child(section_heading(format!(
                            "Background jobs ({})",
                            self.activity.background_jobs.len()
                        )))
                        .children(self.activity.background_jobs.iter().map(background_job_row)),
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
            .gap(theme().space.md)
            .child(activity)
            .when(self.project.repository.backend.is_some(), |run| {
                run.child(self.render_repository(entity.clone(), run_panel.clone(), browser))
            });
        panel()
            .size_full()
            .rounded_none()
            .border_0()
            .bg(theme().colors.inspector)
            .child(body)
    }
}

fn inspector_section() -> gpui::Div {
    div().flex().flex_col().gap(px(7.0))
}
