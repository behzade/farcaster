use std::time::{Duration, SystemTime};

use gpui::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    Role, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::progress::ProgressCircle;

use super::super::PiApp;
use crate::{
    agent_activity::{AgentActivity, AgentLifecycle, AgentOutcome},
    primitives::{ButtonTone, button, panel, section_heading},
    session_changes::{FileChange, FileChangeKind},
    sessions::{UsageSummary, descendant_sessions, root_session_for_path},
    theme::{MONO_FONT_FAMILY, THEME},
};

const CONTEXT_WARNING_PERCENT: f64 = 80.0;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ContextSummary {
    pub percent: Option<f64>,
    pub used: Option<u64>,
    pub total: Option<u64>,
    pub remaining: Option<u64>,
    pub cost_micros: u64,
    pub warning: bool,
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
            root.is_none_or(|root| {
                self.snapshot.selected_session.as_deref() == Some(root.path.as_path())
            })
            .then_some(&self.snapshot.stats),
            aggregate.cost_micros,
        );
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
            if self.context_details_expanded {
                "Hide details"
            } else {
                "Details"
            },
            "Context details",
            self.context_details_expanded,
            RunDisclosure::Context,
            entity.clone(),
        );
        let completed_control = disclosure_control(
            "toggle-completed-agents",
            if self.completed_agents_expanded {
                "Collapse"
            } else {
                "Show"
            },
            "Completed agents",
            self.completed_agents_expanded,
            RunDisclosure::Completed,
            entity.clone(),
        );
        let limited_control = disclosure_control(
            "toggle-limited-agents",
            if self.limited_agents_expanded {
                "Collapse"
            } else {
                "Show"
            },
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
            .gap(THEME.space.md)
            .overflow_y_scroll()
            .child(section_heading("Context"))
            .child(render_context_summary(&context, context_control))
            .when(self.context_details_expanded, |run| {
                run.child(render_accounting(
                    aggregate,
                    self.snapshot.conversation.latest_cache_hit_rate,
                ))
            })
            .when_some(self.fps_monitor.clone(), |run, monitor| run.child(monitor))
            .when(!active.is_empty(), |run| {
                run.child(section_heading("Active agents"))
                    .children(active.iter().filter_map(|(activity, depth, _)| {
                        self.agent_card(activity, *depth, true, false, entity.clone())
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
                    run.children(completed.iter().filter_map(|(activity, depth, _)| {
                        self.agent_card(activity, *depth, false, false, entity.clone())
                    }))
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
                        self.agent_card(activity, *depth, false, true, entity.clone())
                    }))
                })
            });
        panel().size_full().rounded_none().border_0().child(body)
    }

    fn render_changes(&self, entity: WeakEntity<Self>) -> AnyElement {
        if let Some(error) = &self.changes.set.unavailable {
            return div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.warning)
                .child(error.clone())
                .into_any_element();
        }
        if self.changes.set.files.is_empty() {
            return div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child("No observed files differ from HEAD")
                .into_any_element();
        }
        let additions = self
            .changes
            .set
            .files
            .iter()
            .filter_map(|file| file.additions)
            .sum::<u64>();
        let deletions = self
            .changes
            .set
            .files
            .iter()
            .filter_map(|file| file.deletions)
            .sum::<u64>();
        div()
            .flex()
            .flex_col()
            .gap(THEME.space.xs)
            .child(
                div()
                    .font_family(MONO_FONT_FAMILY)
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.subtle)
                    .child(format!(
                        "{} files  +{additions}  -{deletions}",
                        self.changes.set.files.len()
                    )),
            )
            .children(
                self.changes
                    .set
                    .files
                    .iter()
                    .filter_map(|file| self.change_row(file, entity.clone())),
            )
            .into_any_element()
    }

    fn change_row(&self, file: &FileChange, entity: WeakEntity<Self>) -> Option<AnyElement> {
        let focus = self.changes.row_focus.get(&file.path)?.clone();
        let keyboard_focus = focus.clone();
        let click_focus = focus.clone();
        let keyboard_file = file.clone();
        let click_file = file.clone();
        let keyboard_entity = entity.clone();
        let path = file.path.to_string_lossy().into_owned();
        let display_path = if let Some(old) = &file.old_path {
            format!("{} → {}", old.display(), file.path.display())
        } else {
            path.clone()
        };
        let display_path = middle_truncate(&display_path, 52);
        let state = match file.kind {
            FileChangeKind::Modified => "M",
            FileChangeKind::Added => "A",
            FileChangeKind::Deleted => "D",
            FileChangeKind::Renamed => "R",
            FileChangeKind::Binary => "B",
            FileChangeKind::Unavailable => "?",
        };
        let counts = match (file.kind, file.additions, file.deletions) {
            (FileChangeKind::Binary, _, _) => "binary".into(),
            (_, Some(add), Some(del)) => format!("+{add} -{del}"),
            (FileChangeKind::Renamed, _, _) => "rename".into(),
            _ => "counts unavailable".into(),
        };
        Some(
            div()
                .id(format!("change-row-{path}"))
                .track_focus(&focus)
                .role(Role::Button)
                .aria_label(format!("Open diff for {path}"))
                .tab_index(0)
                .px(THEME.space.xs)
                .py(px(4.0))
                .flex()
                .items_center()
                .gap(THEME.space.xs)
                .border(THEME.border)
                .border_color(THEME.colors.border)
                .hover(|row| row.bg(THEME.colors.hover))
                .focus(|row| row.border_color(THEME.colors.accent))
                .cursor_pointer()
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        window.prevent_default();
                        let _ = keyboard_entity.update(cx, |this, cx| {
                            this.open_file_diff(
                                keyboard_file.clone(),
                                keyboard_focus.clone(),
                                window,
                                cx,
                            )
                        });
                    }
                })
                .on_click(move |_, window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.open_file_diff(click_file.clone(), click_focus.clone(), window, cx)
                    });
                })
                .child(
                    div()
                        .w(px(14.0))
                        .font_family(MONO_FONT_FAMILY)
                        .text_color(THEME.colors.accent)
                        .child(state),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_family(MONO_FONT_FAMILY)
                        .text_size(THEME.type_scale.caption)
                        .child(display_path),
                )
                .child(
                    div()
                        .flex_none()
                        .font_family(MONO_FONT_FAMILY)
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child(counts),
                )
                .into_any_element(),
        )
    }

    fn agent_card(
        &self,
        activity: &AgentActivity,
        depth: usize,
        active: bool,
        limited: bool,
        entity: WeakEntity<Self>,
    ) -> Option<AnyElement> {
        let focus = self.agent_row_focus.get(&activity.session_id)?.clone();
        let keyboard_focus = focus.clone();
        let click_focus = focus.clone();
        let keyboard_id = activity.session_id.clone();
        let click_id = activity.session_id.clone();
        let keyboard_entity = entity.clone();
        let displayed_lifecycle = if limited {
            AgentLifecycle::Unknown
        } else {
            activity.lifecycle
        };
        let state = lifecycle_label(displayed_lifecycle);
        let activity_text = activity.activity.clone();
        let role = activity.role.clone();
        let marker = role_glyph(&role);
        let tool = active
            .then(|| {
                activity
                    .current_tool
                    .as_ref()
                    .map(|tool| ("Current", tool))
                    .or_else(|| activity.recent_tool.as_ref().map(|tool| ("Recent", tool)))
            })
            .flatten()
            .map(|(timing, tool)| {
                let mut detail = if tool.target.is_empty() {
                    tool.name.clone()
                } else {
                    format!("{} · {}", tool.name, tool.target)
                };
                if tool.failed {
                    detail.push_str(" · failed");
                }
                format!("{timing}: {detail}")
            });
        let facts = format!(
            "{} · {} tokens · {}",
            tool_count(activity),
            compact_number(activity.usage.total),
            elapsed_label(activity, SystemTime::now())
        );
        Some(
            div()
                .id(format!("agent-card-{}", activity.session_id))
                .track_focus(&focus)
                .role(Role::Button)
                .aria_label(format!("Open {role} agent details: {state}"))
                .tab_index(0)
                .ml(px(depth.saturating_sub(1) as f32 * 8.0))
                .p(THEME.space.sm)
                .border(THEME.border)
                .border_color(THEME.colors.border)
                .bg(THEME.colors.canvas)
                .flex()
                .gap(THEME.space.sm)
                .hover(|card| card.bg(THEME.colors.hover))
                .focus(|card| card.border_color(THEME.colors.accent))
                .cursor_pointer()
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        window.prevent_default();
                        let _ = keyboard_entity.update(cx, |this, cx| {
                            this.open_agent_detail(
                                keyboard_id.clone(),
                                keyboard_focus.clone(),
                                window,
                                cx,
                            );
                        });
                    }
                })
                .on_click(move |_, window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.open_agent_detail(click_id.clone(), click_focus.clone(), window, cx);
                    });
                })
                .child(
                    div()
                        .size(px(28.0))
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
                                .max_h(px(36.0))
                                .overflow_hidden()
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.muted)
                                .child(if activity_text.is_empty() {
                                    "No assigned activity".into()
                                } else {
                                    activity_text
                                }),
                        )
                        .children(tool.map(|tool| {
                            div()
                                .font_family(MONO_FONT_FAMILY)
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.subtle)
                                .child(tool)
                        }))
                        .child(
                            div()
                                .font_family(MONO_FONT_FAMILY)
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.subtle)
                                .child(facts),
                        ),
                )
                .into_any_element(),
        )
    }

    fn open_agent_detail(
        &mut self,
        id: String,
        opener: gpui::FocusHandle,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        opener.focus(window, cx);
        self.agent_detail_return_focus = Some(opener);
        self.agent_detail = Some(id);
        self.pending_agent_detail_setup = true;
        cx.notify();
    }

    pub(crate) fn close_agent_detail(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.agent_detail = None;
        self.pending_agent_detail_setup = false;
        let focus = self
            .agent_detail_return_focus
            .take()
            .unwrap_or_else(|| self.composer_focus.clone());
        focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn render_agent_detail(&self, entity: WeakEntity<Self>) -> Option<AnyElement> {
        let id = self.agent_detail.as_deref()?;
        let activity = self.agent_activities.get(id)?;
        let tool = activity
            .current_tool
            .as_ref()
            .or(activity.recent_tool.as_ref());
        Some(
            div()
                .size_full()
                .p(THEME.space.md)
                .flex()
                .flex_col()
                .gap(THEME.space.md)
                .child(section_heading("Agent details"))
                .child(metric_row("Role", activity.role.clone()))
                .child(metric_row(
                    "State",
                    lifecycle_label(if activity.limited {
                        AgentLifecycle::Unknown
                    } else {
                        activity.lifecycle
                    })
                    .into(),
                ))
                .child(metric_row("Activity", activity.activity.clone()))
                .children(tool.map(|tool| {
                    metric_row(
                        if activity.current_tool.is_some() {
                            "Current tool"
                        } else {
                            "Recent tool"
                        },
                        if tool.target.is_empty() {
                            tool.name.clone()
                        } else {
                            format!("{} · {}", tool.name, tool.target)
                        },
                    )
                }))
                .child(metric_row("Tool calls", tool_count(activity)))
                .child(metric_row("Tokens", compact_number(activity.usage.total)))
                .child(metric_row(
                    "Elapsed",
                    elapsed_label(activity, SystemTime::now()),
                ))
                .when(!activity.changed_paths.is_empty(), |detail| {
                    detail.child(section_heading("Observed changes")).children(
                        activity.changed_paths.iter().map(|observed| {
                            div()
                                .font_family(MONO_FONT_FAMILY)
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.muted)
                                .child(observed.path.to_string_lossy().into_owned())
                        }),
                    )
                })
                .child(button(
                    "close-agent-details",
                    "Close",
                    ButtonTone::Neutral,
                    true,
                    move |window, cx| {
                        let _ = entity.update(cx, |this, cx| this.close_agent_detail(window, cx));
                    },
                ))
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
    aria_label: &'static str,
    expanded: bool,
    disclosure: RunDisclosure,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let keyboard_entity = entity.clone();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(format!(
            "{} {aria_label}",
            if expanded { "Collapse" } else { "Expand" }
        ))
        .aria_expanded(expanded)
        .tab_index(0)
        .px(THEME.space.sm)
        .py(px(3.0))
        .rounded(THEME.radius)
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .text_size(THEME.type_scale.caption)
        .text_color(THEME.colors.muted)
        .hover(|control| control.bg(THEME.colors.hover))
        .focus(|control| control.border_color(THEME.colors.accent))
        .cursor_pointer()
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                window.prevent_default();
                let _ = keyboard_entity
                    .update(cx, |this, cx| this.toggle_run_disclosure(disclosure, cx));
            }
        })
        .on_click(move |_, _, cx| {
            let _ = entity.update(cx, |this, cx| this.toggle_run_disclosure(disclosure, cx));
        })
        .child(label)
        .into_any_element()
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

fn render_context_summary(summary: &ContextSummary, control: AnyElement) -> impl IntoElement {
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
                        .text_size(px(18.0))
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
                .child(control),
        )
}

fn render_accounting(usage: UsageSummary, cache_hit_rate: Option<f64>) -> impl IntoElement {
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
        .child(metric_row(
            "Latest cache hit",
            cache_hit_rate.map_or_else(|| "—".into(), |rate| format!("{rate:.1}%")),
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

fn middle_truncate(value: &str, max_chars: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars || max_chars < 3 {
        return value.to_owned();
    }
    let left = (max_chars - 1) / 2;
    let right = max_chars - 1 - left;
    chars[..left]
        .iter()
        .chain(std::iter::once(&'…'))
        .chain(chars[chars.len() - right..].iter())
        .collect()
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
