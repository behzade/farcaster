use gpui::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    Role, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _, px, uniform_list,
};

use super::super::PiApp;
use crate::{
    primitives::{ButtonTone, button, panel, section_heading},
    sessions::{SessionSummary, UsageSummary, descendant_sessions, root_session_for_path},
    theme::THEME,
};

impl PiApp {
    pub(super) fn render_narrow_navigation(&self, entity: WeakEntity<Self>) -> impl IntoElement {
        let sessions_entity = entity.clone();
        div()
            .h(px(40.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px(THEME.space.sm)
            .border_b(THEME.border)
            .border_color(THEME.colors.border)
            .bg(THEME.colors.panel)
            .child(button(
                "open-sessions",
                "Sessions",
                ButtonTone::Quiet,
                true,
                move |window, cx| {
                    let _ =
                        sessions_entity.update(cx, |this, cx| this.open_sessions_sheet(window, cx));
                },
            ))
            .child(button(
                "open-run",
                "Session details",
                ButtonTone::Quiet,
                true,
                move |window, cx| {
                    let _ = entity.update(cx, |this, cx| this.open_run_sheet(window, cx));
                },
            ))
    }

    pub(super) fn render_run_panel(&self, entity: WeakEntity<Self>) -> impl IntoElement {
        let root = root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref());
        let descendants = root
            .map(|root| descendant_sessions(&self.sessions, &root.id))
            .unwrap_or_default();
        let show_main_context = root.is_none_or(|root| {
            self.snapshot.selected_session.as_deref() == Some(root.path.as_path())
        });
        let mut aggregate_usage = root.map(|root| root.usage).unwrap_or_default();
        for (session, _) in &descendants {
            aggregate_usage.add(session.usage);
        }
        let mut agent_rows = Vec::new();
        if let Some(root) = root {
            let root_status = if self.snapshot.live_session.as_deref() == Some(root.path.as_path())
            {
                normalized_agent_status(&self.snapshot.live_status)
            } else if root.is_running {
                "Active"
            } else {
                "Done"
            };
            agent_rows.push((
                root.clone(),
                0,
                "Main".into(),
                self.snapshot.selected_session.as_deref() == Some(root.path.as_path()),
                root_status.to_owned(),
            ));
            agent_rows.extend(descendants.into_iter().map(|(session, depth)| {
                (
                    session.clone(),
                    depth,
                    compact_subagent_label(&session.title),
                    self.snapshot.selected_session.as_deref() == Some(session.path.as_path()),
                    if session.is_running { "Active" } else { "Done" }.into(),
                )
            }));
        }
        let agent_count = agent_rows.len();
        let agent_height =
            px((agent_count.min(7) as f32) * f32::from(THEME.layout.agent_row_height));
        let agent_entity = entity.clone();
        let agent_list = uniform_list("agent-session-list", agent_count, move |range, _, _| {
            range
                .filter_map(|index| agent_rows.get(index))
                .map(|(session, depth, label, selected, status)| {
                    agent_session_row(
                        session,
                        *depth,
                        label.clone(),
                        *selected,
                        status.clone(),
                        agent_entity.clone(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .w_full()
        .h(agent_height)
        .max_h(THEME.layout.agent_list_max_height);
        let body = div()
            .id("run-panel-scroll")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .p(THEME.space.sm)
            .gap(THEME.space.md)
            .overflow_y_scroll()
            .child(section_heading("Usage"))
            .child(usage_metrics(
                show_main_context.then_some(&self.snapshot.stats),
                aggregate_usage,
                self.snapshot.conversation.latest_cache_hit_rate,
            ))
            .when_some(self.fps_monitor.clone(), |run, monitor| run.child(monitor))
            .when(agent_count > 0, |run| {
                run.child(section_heading("Agents"))
                    .child(div().overflow_y_hidden().child(agent_list))
            });
        panel().size_full().rounded_none().border_0().child(body)
    }
}

fn agent_session_row(
    session: &SessionSummary,
    depth: usize,
    label: String,
    selected: bool,
    status: String,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let path = session.path.clone();
    let project = session.project.clone();
    let keyboard_path = path.clone();
    let keyboard_project = project.clone();
    let keyboard_entity = entity.clone();
    let title = session.title.clone();
    let details = format!("{status} · {}", compact_number(session.usage.total));
    let status_is_active = status != "Done";
    div()
        .id(format!("agent-session-{}", session.id))
        .role(Role::Button)
        .aria_label(format!("Open agent session: {title} ({status})"))
        .aria_selected(selected)
        .tab_index(0)
        .flex()
        .items_center()
        .gap(THEME.space.sm)
        .pl(px(8.0 + depth as f32 * 12.0))
        .pr(THEME.space.xs)
        .h(THEME.layout.agent_row_height)
        .border_b(THEME.border)
        .border_color(THEME.colors.border)
        .when(selected, |row| {
            row.border_l(px(2.0)).border_color(THEME.colors.accent)
        })
        .hover(|row| row.bg(THEME.colors.hover))
        .focus(|row| row.border(THEME.border).border_color(THEME.colors.accent))
        .cursor_pointer()
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                window.prevent_default();
                let _ = keyboard_entity.update(cx, |this, cx| {
                    this.resume(keyboard_path.clone(), keyboard_project.clone(), window, cx)
                });
            }
        })
        .on_click(move |_, window, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.resume(path.clone(), project.clone(), window, cx)
            });
        })
        .child(
            div()
                .min_w_0()
                .flex_1()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.text)
                .child(label),
        )
        .child(
            div()
                .flex_none()
                .text_size(THEME.type_scale.caption)
                .text_color(if status_is_active {
                    THEME.colors.accent
                } else {
                    THEME.colors.subtle
                })
                .child(details),
        )
        .into_any_element()
}

fn usage_metrics(
    stats: Option<&serde_json::Value>,
    usage: UsageSummary,
    latest_cache_hit_rate: Option<f64>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(metric_row(
            "Main context",
            stats.map_or_else(|| "—".into(), context_usage),
        ))
        .child(metric_row("Tokens", compact_number(usage.total)))
        .child(metric_row("Input", compact_number(usage.input)))
        .child(metric_row("Output", compact_number(usage.output)))
        .child(metric_row(
            "Cache",
            compact_number(usage.cache_read.saturating_add(usage.cache_write)),
        ))
        .child(metric_row(
            "Cache hit rate",
            format_cache_hit_rate(latest_cache_hit_rate),
        ))
        .child(metric_row("Cost", format_cost(usage.cost_micros)))
}

fn metric_row(label: &'static str, value: String) -> impl IntoElement {
    div()
        .h(THEME.layout.status_row_height)
        .flex()
        .items_center()
        .justify_between()
        .gap(THEME.space.sm)
        .text_size(THEME.type_scale.caption)
        .child(div().text_color(THEME.colors.subtle).child(label))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_weight(FontWeight::MEDIUM)
                .text_color(THEME.colors.muted)
                .child(value),
        )
}

fn context_usage(stats: &serde_json::Value) -> String {
    let Some(context) = stats.get("contextUsage") else {
        return "—".into();
    };
    let tokens = context.get("tokens").and_then(serde_json::Value::as_u64);
    let window = context
        .get("contextWindow")
        .and_then(serde_json::Value::as_u64);
    let percent = context.get("percent").and_then(serde_json::Value::as_f64);
    match (tokens, window, percent) {
        (Some(tokens), Some(window), Some(percent)) => format!(
            "{} / {} · {percent:.0}%",
            compact_number(tokens),
            compact_number(window)
        ),
        (Some(tokens), Some(window), None) => {
            format!("{} / {}", compact_number(tokens), compact_number(window))
        }
        (Some(tokens), None, _) => compact_number(tokens),
        _ => "—".into(),
    }
}

fn compact_number(value: u64) -> String {
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

fn format_cache_hit_rate(rate: Option<f64>) -> String {
    rate.map_or_else(|| "—".into(), |rate| format!("{rate:.1}%"))
}

fn format_cost(micros: u64) -> String {
    let dollars = micros as f64 / 1_000_000.0;
    if micros == 0 {
        "$0".into()
    } else if dollars < 0.01 {
        format!("${dollars:.4}")
    } else {
        format!("${dollars:.2}")
    }
}

fn bounded_label(value: &str, max: usize) -> String {
    let mut label = value.chars().take(max).collect::<String>();
    if value.chars().count() > max {
        label.push('…');
    }
    label
}

fn normalized_agent_status(status: &str) -> &str {
    if matches!(status, "" | "Done" | "Idle" | "Ready") {
        "Done"
    } else {
        status
    }
}

fn compact_subagent_label(value: &str) -> String {
    let Some(generated) = value.strip_prefix("subagent-") else {
        return bounded_label(value, 24);
    };
    let Some((role, _)) = generated.split_once('-') else {
        return bounded_label(value, 24);
    };
    if role.is_empty() {
        return bounded_label(value, 24);
    }
    generated
        .rsplit('-')
        .next()
        .filter(|suffix| suffix.chars().all(|character| character.is_ascii_digit()))
        .map_or_else(|| role.to_owned(), |suffix| format!("{role} {suffix}"))
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
