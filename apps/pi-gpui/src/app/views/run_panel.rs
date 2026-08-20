use std::time::{Duration, SystemTime};

use gpui::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};

use super::super::PiApp;
use crate::{
    agent_activity::{AgentActivity, AgentLifecycle, AgentOutcome},
    assets::AppIcon,
    primitives::{
        AppIconSize, app_icon, disclosure_button, panel, section_heading, spinning_app_icon,
    },
    sessions::{descendant_sessions, root_session_for_path},
    theme::THEME,
};

const MAX_VISIBLE_COMPLETED_AGENTS: usize = 5;

impl PiApp {
    pub(super) fn render_run_panel(&self, entity: WeakEntity<Self>) -> impl IntoElement {
        let root = root_session_for_path(
            &self.all_sessions,
            self.snapshot.selected_session.as_deref(),
        );
        let descendants = root
            .map(|root| descendant_sessions(&self.all_sessions, &root.id))
            .unwrap_or_default();
        let main = root.and_then(|session| {
            self.agent_activities
                .get(&session.id)
                .map(|activity| (activity, session))
        });
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
            self.completed_agents_expanded,
            RunDisclosure::Completed,
            entity.clone(),
        );
        let limited_control = disclosure_control(
            "toggle-limited-agents",
            "Limited agents",
            self.limited_agents_expanded,
            RunDisclosure::Limited,
            entity.clone(),
        );
        let body = div()
            .id("run-panel-scroll")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .p(THEME.space.sm)
            .gap(THEME.space.sm)
            .overflow_y_scroll()
            .track_scroll(&self.run_panel_scroll)
            .child(self.workgraph_sidebar_view.clone())
            .when_some(self.fps_monitor.clone(), |run, monitor| run.child(monitor))
            .when_some(self.performance_monitor.as_ref(), |run, monitor| {
                run.child(render_performance(&monitor.summary))
            })
            .when(main.is_some() || !active.is_empty(), |run| {
                run.child(section_heading("Agents"))
                    .children(main.and_then(|(activity, session)| {
                        self.agent_card(
                            activity,
                            0,
                            matches!(
                                agent_section(
                                    activity.lifecycle,
                                    activity.limited,
                                    session.is_running
                                ),
                                AgentSection::Active
                            ),
                            false,
                            Some("Main"),
                            entity.clone(),
                        )
                    }))
                    .children(active.iter().filter_map(|(activity, depth, _)| {
                        self.agent_card(activity, *depth, true, false, None, entity.clone())
                    }))
            })
            .child(section_heading("Changes"))
            .child(self.render_changes(entity.clone()))
            .when(!completed.is_empty(), |run| {
                run.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(section_heading(format!(
                            "Completed agents ({})",
                            completed.len()
                        )))
                        .child(completed_control),
                )
                .when(self.completed_agents_expanded, |run| {
                    run.children(
                        completed
                            .iter()
                            .take(MAX_VISIBLE_COMPLETED_AGENTS)
                            .filter_map(|(activity, depth, _)| {
                                self.agent_card(
                                    activity,
                                    *depth,
                                    false,
                                    false,
                                    None,
                                    entity.clone(),
                                )
                            }),
                    )
                    .when(
                        completed.len() > MAX_VISIBLE_COMPLETED_AGENTS,
                        |run| {
                            run.child(
                                div()
                                    .text_size(THEME.type_scale.caption)
                                    .text_color(THEME.colors.subtle)
                                    .child(format!(
                                        "Showing the {} most recent completed agents",
                                        MAX_VISIBLE_COMPLETED_AGENTS
                                    )),
                            )
                        },
                    )
                })
            })
            .when(!limited.is_empty(), |run| {
                run.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(section_heading(format!(
                            "Limited agents ({})",
                            limited.len()
                        )))
                        .child(limited_control),
                )
                .when(self.limited_agents_expanded, |run| {
                    run.children(limited.iter().filter_map(|(activity, depth, _)| {
                        self.agent_card(activity, *depth, false, true, None, entity.clone())
                    }))
                })
            });
        panel().size_full().rounded_none().border_0().child(body)
    }

    fn agent_card(
        &self,
        activity: &AgentActivity,
        depth: usize,
        active: bool,
        limited: bool,
        role_override: Option<&str>,
        entity: WeakEntity<Self>,
    ) -> Option<AnyElement> {
        let focus = self.agent_row_focus.get(&activity.session_id)?.clone();
        let session = self
            .all_sessions
            .iter()
            .find(|session| session.id == activity.session_id)?;
        let path = session.path.clone();
        let project = session.project.clone();
        let displayed_lifecycle = if limited {
            AgentLifecycle::Unknown
        } else {
            activity.lifecycle
        };
        let state = lifecycle_label(displayed_lifecycle);
        let activity_text = activity.activity.clone();
        let role = role_override.unwrap_or(&activity.role).to_owned();
        let marker = role_glyph(&role);
        let elapsed = elapsed_label(activity, SystemTime::now());
        Some(
            div()
                .id(format!("agent-card-{}", activity.session_id))
                .track_focus(&focus)
                .role(Role::Button)
                .aria_label(format!("Show {role} transcript: {state}"))
                .tab_index(0)
                .ml(px(depth.saturating_sub(1) as f32 * 8.0))
                .px(THEME.space.sm)
                .py(THEME.space.xs)
                .flex()
                .items_stretch()
                .gap(THEME.space.sm)
                .hover(|card| card.bg(THEME.colors.hover))
                .focus(|card| card.bg(THEME.colors.hover))
                .cursor_pointer()
                .on_click(move |_, window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.select_session(path.clone(), project.clone(), window, cx);
                    });
                })
                .child(
                    div()
                        .size(THEME.controls.agent_marker)
                        .flex_none()
                        .rounded_full()
                        .border(THEME.border)
                        .border_color(if active {
                            THEME.colors.accent
                        } else {
                            THEME.colors.border
                        })
                        .flex()
                        .items_center()
                        .justify_center()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if active {
                            THEME.colors.accent
                        } else {
                            THEME.colors.muted
                        })
                        .child(marker),
                )
                .child(
                    div()
                        .w_0()
                        .min_w_0()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_normal()
                        .line_clamp(2)
                        .line_height(THEME.type_scale.line_body)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(THEME.colors.text)
                        .child(if activity_text.is_empty() {
                            role
                        } else {
                            activity_text
                        }),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_start()
                        .justify_end()
                        .gap(px(3.0))
                        .text_size(THEME.type_scale.caption)
                        .whitespace_nowrap()
                        .text_color(lifecycle_color(displayed_lifecycle))
                        .child(lifecycle_indicator(
                            displayed_lifecycle,
                            &activity.session_id,
                        ))
                        .child(elapsed),
                )
                .into_any_element(),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AgentSection {
    Active,
    Completed,
    Limited,
    Hidden,
}

fn agent_section(lifecycle: AgentLifecycle, limited: bool, is_running: bool) -> AgentSection {
    if is_running
        && matches!(
            lifecycle,
            AgentLifecycle::NeedsInput | AgentLifecycle::Working
        )
    {
        return AgentSection::Active;
    }
    if limited || matches!(lifecycle, AgentLifecycle::Unknown) {
        return AgentSection::Limited;
    }
    match lifecycle {
        AgentLifecycle::NeedsInput | AgentLifecycle::Working if is_running => AgentSection::Active,
        AgentLifecycle::Completed(_) => AgentSection::Completed,
        AgentLifecycle::NeedsInput | AgentLifecycle::Working | AgentLifecycle::Unknown => {
            AgentSection::Hidden
        }
    }
}

#[derive(Clone, Copy)]
enum RunDisclosure {
    Completed,
    Limited,
}

fn disclosure_control(
    id: &'static str,
    label: &'static str,
    expanded: bool,
    disclosure: RunDisclosure,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    disclosure_button(id, expanded, label, move |_, cx| {
        let _ = entity.update(cx, |this, cx| this.toggle_run_disclosure(disclosure, cx));
    })
}

impl PiApp {
    fn toggle_run_disclosure(&mut self, disclosure: RunDisclosure, cx: &mut gpui::Context<Self>) {
        match disclosure {
            RunDisclosure::Completed => {
                self.completed_agents_expanded = !self.completed_agents_expanded
            }
            RunDisclosure::Limited => self.limited_agents_expanded = !self.limited_agents_expanded,
        }
        self.notify_run_panel(cx);
    }
}

fn render_performance(summary: &crate::performance::PerformanceSummary) -> impl IntoElement {
    div()
        .p(THEME.space.sm)
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.canvas)
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(section_heading("GPUI profiler · 1 second"))
        .child(metric_row(
            "Frames",
            format!("{} sampled", summary.frame_count),
        ))
        .child(metric_row(
            "Draw p95 / max",
            format!(
                "{} / {}",
                crate::performance::duration_label(summary.draw_p95),
                crate::performance::duration_label(summary.draw_max)
            ),
        ))
        .child(metric_row(
            "Dirty to draw p95",
            crate::performance::duration_label(summary.dirty_to_draw_p95),
        ))
        .child(metric_row(
            "Invalidations avg / max",
            format!(
                "{:.1} / {}",
                summary.invalidations_average, summary.invalidations_max
            ),
        ))
        .child(metric_row(
            "Snapshots / coalesced",
            format!(
                "{} / {}",
                summary.snapshots_published, summary.stream_events_coalesced
            ),
        ))
        .child(metric_row(
            "Transcript examined / remeasured",
            format!(
                "{} / {}",
                summary.transcript_items_examined, summary.transcript_rows_remeasured
            ),
        ))
        .child(metric_row(
            "Catalog parses / highlight bytes",
            format!(
                "{} / {}",
                summary.catalog_files_parsed, summary.highlight_bytes
            ),
        ))
        .child(metric_row(
            "Slowest task poll",
            summary.slowest_task.clone().unwrap_or_else(|| "—".into()),
        ))
        .child(metric_row(
            "Slowest action",
            summary.slowest_action.clone().unwrap_or_else(|| "—".into()),
        ))
}

fn metric_row(label: &'static str, value: String) -> impl IntoElement {
    div()
        .min_h(THEME.layout.status_row_height)
        .flex()
        .items_center()
        .justify_between()
        .gap(THEME.space.sm)
        .text_size(THEME.type_scale.caption)
        .child(div().text_color(THEME.colors.subtle).child(label))
        .child(
            div()
                .min_w_0()
                .text_align(gpui::TextAlign::Right)
                .font_weight(FontWeight::MEDIUM)
                .text_color(THEME.colors.muted)
                .child(value),
        )
}

fn role_glyph(role: &str) -> String {
    match role.to_ascii_lowercase().as_str() {
        "reviewer" => "✓",
        "scout" => "⌕",
        "researcher" => "◎",
        "worker" => "⚒",
        _ => "◆",
    }
    .into()
}

fn lifecycle_label(lifecycle: AgentLifecycle) -> &'static str {
    match lifecycle {
        AgentLifecycle::NeedsInput => "Needs input",
        AgentLifecycle::Working => "Working",
        AgentLifecycle::Unknown => "Unknown",
        AgentLifecycle::Completed(AgentOutcome::Complete) => "Complete",
        AgentLifecycle::Completed(AgentOutcome::Failed) => "Failed",
        AgentLifecycle::Completed(AgentOutcome::Incomplete) => "Incomplete",
    }
}

fn lifecycle_icon(lifecycle: AgentLifecycle) -> AppIcon {
    match lifecycle {
        AgentLifecycle::NeedsInput | AgentLifecycle::Completed(AgentOutcome::Incomplete) => {
            AppIcon::WarningCircle
        }
        AgentLifecycle::Working => AppIcon::SpinnerGap,
        AgentLifecycle::Unknown => AppIcon::Question,
        AgentLifecycle::Completed(AgentOutcome::Complete) => AppIcon::CheckCircle,
        AgentLifecycle::Completed(AgentOutcome::Failed) => AppIcon::XCircle,
    }
}

fn lifecycle_indicator(lifecycle: AgentLifecycle, session_id: &str) -> AnyElement {
    let icon = app_icon(lifecycle_icon(lifecycle), AppIconSize::Inline);
    if matches!(lifecycle, AgentLifecycle::Working) {
        spinning_app_icon(
            lifecycle_icon(lifecycle),
            AppIconSize::Inline,
            format!("agent-spinner-{session_id}"),
        )
    } else {
        icon.into_any_element()
    }
}

fn lifecycle_color(lifecycle: AgentLifecycle) -> gpui::Rgba {
    match lifecycle {
        AgentLifecycle::NeedsInput => THEME.colors.warning,
        AgentLifecycle::Working => THEME.colors.accent,
        AgentLifecycle::Completed(AgentOutcome::Failed) => THEME.colors.error,
        AgentLifecycle::Completed(AgentOutcome::Incomplete) => THEME.colors.warning,
        AgentLifecycle::Completed(AgentOutcome::Complete) => THEME.colors.success,
        AgentLifecycle::Unknown => THEME.colors.subtle,
    }
}

fn elapsed_label(activity: &AgentActivity, now: SystemTime) -> String {
    let duration = activity.elapsed.or_else(|| {
        matches!(
            activity.lifecycle,
            AgentLifecycle::NeedsInput | AgentLifecycle::Working
        )
        .then(|| now.duration_since(activity.started).ok())
        .flatten()
    });
    format_duration(duration)
}

fn format_duration(duration: Option<Duration>) -> String {
    let Some(duration) = duration else {
        return "—".into();
    };
    let seconds = duration.as_secs();
    if seconds >= 3_600 {
        format!("{}h {}m", seconds / 3_600, seconds % 3_600 / 60)
    } else if seconds >= 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
#[path = "run_panel_tests.rs"]
mod tests;
