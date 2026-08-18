use std::time::{Duration, SystemTime};

use gpui::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};
use gpui_component::progress::ProgressCircle;

use super::super::PiApp;
use crate::{
    agent_activity::{AgentActivity, AgentLifecycle, AgentOutcome},
    primitives::{disclosure_button, panel, section_heading},
    sessions::{UsageSummary, descendant_sessions, root_session_for_path},
    theme::{MONO_FONT_FAMILY, THEME},
};

const CONTEXT_WARNING_PERCENT: f64 = 80.0;
const MAX_VISIBLE_COMPLETED_AGENTS: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ContextSummary {
    pub percent: Option<f64>,
    pub used: Option<u64>,
    pub total: Option<u64>,
    pub remaining: Option<u64>,
    pub cost_micros: u64,
    pub warning: bool,
}

fn visible_context_stats(stats: &serde_json::Value, running: bool) -> Option<&serde_json::Value> {
    let context = stats.get("contextUsage");
    let meaningful = context
        .and_then(|context| context.get("tokens"))
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|tokens| tokens > 0)
        || context
            .and_then(|context| context.get("percent"))
            .and_then(serde_json::Value::as_f64)
            .is_some_and(|percent| percent.is_finite() && percent > 0.0);
    (!running || meaningful).then_some(stats)
}

pub(super) fn context_summary(
    stats: Option<&serde_json::Value>,
    cost_micros: u64,
) -> ContextSummary {
    let context = stats.and_then(|stats| stats.get("contextUsage"));
    let used = context
        .and_then(|context| context.get("tokens"))
        .and_then(serde_json::Value::as_u64);
    let total = context
        .and_then(|context| context.get("contextWindow"))
        .and_then(serde_json::Value::as_u64)
        .filter(|total| *total > 0);
    let percent = context
        .and_then(|context| context.get("percent"))
        .and_then(serde_json::Value::as_f64)
        .filter(|percent| percent.is_finite())
        .or_else(|| match (used, total) {
            (Some(used), Some(total)) => Some(used as f64 * 100.0 / total as f64),
            _ => None,
        })
        .map(|percent| percent.clamp(0.0, 100.0));
    ContextSummary {
        percent,
        used,
        total,
        remaining: used
            .zip(total)
            .map(|(used, total)| total.saturating_sub(used)),
        cost_micros,
        warning: percent.is_some_and(|percent| percent >= CONTEXT_WARNING_PERCENT),
    }
}

impl PiApp {
    pub(super) fn render_run_panel(&self, entity: WeakEntity<Self>) -> impl IntoElement {
        let root = root_session_for_path(
            &self.all_sessions,
            self.snapshot.selected_session.as_deref(),
        );
        let descendants = root
            .map(|root| descendant_sessions(&self.all_sessions, &root.id))
            .unwrap_or_default();
        let mut aggregate = root.map(|root| root.usage).unwrap_or_default();
        for (session, _) in &descendants {
            aggregate.add(session.usage);
        }
        let context = context_summary(
            visible_context_stats(&self.snapshot.stats, self.snapshot.conversation.running),
            aggregate.cost_micros,
        );
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
                AgentSection::Active => active.push((activity, depth, session.modified)),
                AgentSection::Completed => completed.push((activity, depth, session.modified)),
                AgentSection::Limited => limited.push((activity, depth, session.modified)),
                AgentSection::Hidden => {}
            }
        }
        active.sort_by_key(|(activity, _, modified)| {
            (
                !matches!(activity.lifecycle, AgentLifecycle::NeedsInput),
                std::cmp::Reverse(*modified),
            )
        });
        completed.sort_by_key(|(_, _, modified)| std::cmp::Reverse(*modified));
        limited.sort_by_key(|(_, _, modified)| std::cmp::Reverse(*modified));

        let context_control = disclosure_control(
            "toggle-context-details",
            "Context details",
            self.context_details_expanded,
            RunDisclosure::Context,
            entity.clone(),
        );
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
            .child(section_heading("Context"))
            .child(render_context_summary(
                &context,
                self.snapshot.conversation.latest_cache_hit_rate,
                context_control,
            ))
            .when(self.context_details_expanded, |run| {
                run.child(render_accounting(aggregate))
            })
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
        let selected = self.snapshot.selected_session.as_deref() == Some(session.path.as_path());
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
        let tool = active
            .then(|| {
                activity
                    .current_tool
                    .as_ref()
                    .or(activity.recent_tool.as_ref())
            })
            .flatten()
            .map(|tool| {
                let mut detail = if tool.target.is_empty() {
                    tool.name.clone()
                } else {
                    format!("{} · {}", tool.name, tool.target)
                };
                if tool.failed {
                    detail.push_str(" · failed");
                }
                detail
            });
        let facts = format!(
            "{} · {} tokens · {}",
            tool_count(activity),
            compact_number(activity.usage.total),
            elapsed_label(activity, SystemTime::now())
        );
        let meta = tool.map_or(facts.clone(), |tool| format!("{facts} · {tool}"));
        Some(
            div()
                .id(format!("agent-card-{}", activity.session_id))
                .track_focus(&focus)
                .role(Role::Button)
                .aria_label(format!("Show {role} transcript: {state}"))
                .tab_index(0)
                .ml(px(depth.saturating_sub(1) as f32 * 8.0))
                .px(THEME.space.xs)
                .py(px(5.0))
                .border(THEME.border)
                .border_color(if selected {
                    THEME.colors.accent
                } else {
                    THEME.colors.border
                })
                .bg(if selected {
                    THEME.colors.surface
                } else {
                    THEME.colors.canvas
                })
                .flex()
                .gap(THEME.space.sm)
                .hover(|card| card.bg(THEME.colors.hover))
                .focus(|card| card.border_color(THEME.colors.accent))
                .cursor_pointer()
                .on_click(move |_, window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.resume(path.clone(), project.clone(), window, cx);
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
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .gap(THEME.space.xs)
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(THEME.colors.text)
                                        .child(role),
                                )
                                .child(
                                    div()
                                        .text_size(THEME.type_scale.caption)
                                        .text_color(lifecycle_color(displayed_lifecycle))
                                        .child(state),
                                ),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.muted)
                                .child(if activity_text.is_empty() {
                                    "No assigned activity".into()
                                } else {
                                    activity_text
                                }),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .font_family(MONO_FONT_FAMILY)
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.subtle)
                                .child(meta),
                        ),
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
    Context,
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
            RunDisclosure::Context => {
                self.context_details_expanded = !self.context_details_expanded
            }
            RunDisclosure::Completed => {
                self.completed_agents_expanded = !self.completed_agents_expanded
            }
            RunDisclosure::Limited => self.limited_agents_expanded = !self.limited_agents_expanded,
        }
        self.notify_run_panel(cx);
    }
}

fn render_context_summary(
    summary: &ContextSummary,
    cache_hit_rate: Option<f64>,
    control: AnyElement,
) -> impl IntoElement {
    let percent = summary
        .percent
        .map_or_else(|| "—".into(), |percent| format!("{percent:.0}%"));
    let used_total = match (summary.used, summary.total) {
        (Some(used), Some(total)) => {
            format!("{} / {}", compact_number(used), compact_number(total))
        }
        (Some(used), None) => compact_number(used),
        _ => "—".into(),
    };
    div()
        .flex()
        .items_center()
        .gap(THEME.space.md)
        .child(
            ProgressCircle::new("context-progress")
                .value(summary.percent.unwrap_or(0.0) as f32)
                .color(if summary.warning {
                    THEME.colors.warning
                } else if summary.percent.is_some() {
                    THEME.colors.accent
                } else {
                    THEME.colors.border
                })
                .size(px(80.0))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(THEME.type_scale.display)
                        .text_color(if summary.warning {
                            THEME.colors.warning
                        } else {
                            THEME.colors.text
                        })
                        .child(percent),
                ),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .gap(THEME.space.xs)
                .child(metric_row("Used / total", used_total))
                .child(metric_row(
                    "Remaining",
                    summary.remaining.map_or_else(|| "—".into(), compact_number),
                ))
                .child(metric_row("Session cost", format_cost(summary.cost_micros)))
                .child(metric_row(
                    "Latest cache hit",
                    cache_hit_rate.map_or_else(|| "—".into(), |rate| format!("{rate:.1}%")),
                ))
                .child(control),
        )
}

fn render_accounting(usage: UsageSummary) -> impl IntoElement {
    div()
        .p(THEME.space.sm)
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.canvas)
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(metric_row("Cumulative input", compact_number(usage.input)))
        .child(metric_row(
            "Cumulative output",
            compact_number(usage.output),
        ))
        .child(metric_row(
            "Cache read / write",
            format!(
                "{} / {}",
                compact_number(usage.cache_read),
                compact_number(usage.cache_write)
            ),
        ))
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

pub(super) fn compact_number(value: u64) -> String {
    if value >= 1_000_000 {
        compact_scaled(value, 1_000_000, "m")
    } else if value >= 1_000 {
        compact_scaled(value, 1_000, "k")
    } else {
        value.to_string()
    }
}

fn compact_scaled(value: u64, scale: u64, suffix: &str) -> String {
    if value.is_multiple_of(scale) {
        format!("{}{suffix}", value / scale)
    } else {
        format!("{:.1}{suffix}", value as f64 / scale as f64)
    }
}

pub(super) fn format_cost(micros: u64) -> String {
    let dollars = micros as f64 / 1_000_000.0;
    if micros == 0 {
        "$0".into()
    } else if dollars < 0.01 {
        format!("${dollars:.4}")
    } else {
        format!("${dollars:.2}")
    }
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

fn tool_count(activity: &AgentActivity) -> String {
    if activity.limited {
        format!("{}+ tools", activity.tool_call_count)
    } else if activity.tool_call_count == 1 {
        "1 tool".into()
    } else {
        format!("{} tools", activity.tool_call_count)
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
