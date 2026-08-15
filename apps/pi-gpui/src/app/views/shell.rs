use std::{
    collections::HashSet,
    path::Path,
    time::{Duration, SystemTime},
};

use gpui::{
    AnyElement, CursorStyle, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Role, StatefulInteractiveElement as _, Styled as _, WeakEntity, accesskit, div,
    prelude::FluentBuilder as _, px, uniform_list,
};
use gpui_component::{Icon, Sizable as _, Size, input::Input};

use super::super::PiApp;
use crate::{
    assets::AppIcon,
    layout::LayoutMode,
    primitives::{ButtonTone, FeedbackTone, button, feedback, icon_button, panel, section_heading},
    runtime::RuntimeCommand,
    sessions::{
        SessionSummary, UsageSummary, descendant_sessions, root_session_for_path, root_sessions,
    },
    theme::THEME,
};

impl PiApp {
    pub(super) fn render_header(
        &self,
        mode: LayoutMode,
        entity: WeakEntity<Self>,
    ) -> impl IntoElement {
        let project_path = if self.snapshot.project.as_os_str().is_empty() {
            &self.project
        } else {
            &self.snapshot.project
        };
        let project = project_path
            .file_name()
            .and_then(|name| name.to_str())
            .map_or_else(|| self.project.display().to_string(), str::to_owned);
        let session_title =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .map(|session| bounded_label(&session.title, 42))
                .unwrap_or_else(|| "New session".into());
        let model = self
            .snapshot
            .session
            .as_ref()
            .and_then(|state| state.model.as_ref())
            .map(|model| bounded_label(&model.name, 28))
            .unwrap_or_else(|| "No model".into());
        let thinking = self
            .snapshot
            .session
            .as_ref()
            .map(|state| state.thinking_level.clone())
            .unwrap_or_else(|| "off".into());
        let status_accessible = if self.snapshot.history_preview {
            "History preview. Pi loads when you send.".into()
        } else {
            self.snapshot.status.clone()
        };
        let model_entity = entity.clone();
        let thinking_entity = entity.clone();
        let sessions_entity = entity.clone();
        let run_entity = entity.clone();
        div()
            .min_h(THEME.layout.header_height)
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap(THEME.space.md)
            .px(THEME.space.md)
            .py(THEME.space.xs)
            .border_b(THEME.border)
            .border_color(THEME.colors.border)
            .bg(THEME.colors.surface)
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .flex_col()
                    .items_start()
                    .gap(THEME.space.xs)
                    .child(
                        div()
                            .max_w(px(420.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(THEME.type_scale.body)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(session_title),
                    )
                    .when(mode != LayoutMode::Narrow, |identity| {
                        identity.child(
                            div()
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.muted)
                                .child(project),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(THEME.space.xs)
                    .when(mode != LayoutMode::Wide, |actions| {
                        actions.child(button(
                            "open-sessions",
                            "Sessions",
                            ButtonTone::Quiet,
                            true,
                            move |window, cx| {
                                let _ = sessions_entity
                                    .update(cx, |this, cx| this.open_sessions_sheet(window, cx));
                            },
                        ))
                    })
                    .when(!self.snapshot.history_preview, |actions| {
                        actions
                            .child(button(
                                "cycle-model",
                                model,
                                ButtonTone::Neutral,
                                !self.snapshot.models.is_empty(),
                                move |_, cx| {
                                    let _ =
                                        model_entity.update(cx, |this, cx| this.cycle_model(cx));
                                },
                            ))
                            .child(button(
                                "cycle-thinking",
                                thinking,
                                ButtonTone::Neutral,
                                !self.snapshot.thinking_levels.is_empty(),
                                move |_, cx| {
                                    let _ = thinking_entity
                                        .update(cx, |this, cx| this.cycle_thinking(cx));
                                },
                            ))
                    })
                    .when(mode != LayoutMode::Wide, |actions| {
                        actions.child(button(
                            "open-run",
                            "Run",
                            ButtonTone::Quiet,
                            true,
                            move |window, cx| {
                                let _ = run_entity
                                    .update(cx, |this, cx| this.open_run_sheet(window, cx));
                            },
                        ))
                    })
                    .child(
                        div()
                            .id("run-status")
                            .role(Role::Status)
                            .a11y_synthetic_children(move |builder| {
                                builder.parent_node().set_live(accesskit::Live::Polite);
                                builder.parent_node().set_value(status_accessible.as_ref());
                            })
                            .when(self.snapshot.history_preview, |status| {
                                status.child(
                                    div().size(px(8.0)).rounded_full().bg(THEME.colors.subtle),
                                )
                            })
                            .when(!self.snapshot.history_preview, |status| {
                                status.child(div().size(px(8.0)).rounded_full().bg(
                                    if self.snapshot.connected {
                                        THEME.colors.accent
                                    } else {
                                        THEME.colors.error
                                    },
                                ))
                            }),
                    ),
            )
    }

    pub(super) fn render_sessions(
        &self,
        collapsed: bool,
        entity: WeakEntity<Self>,
    ) -> impl IntoElement {
        if collapsed {
            return div()
                .size_full()
                .flex()
                .items_start()
                .justify_center()
                .pt(THEME.space.md)
                .text_color(THEME.colors.accent)
                .child("π")
                .into_any_element();
        }
        let new_entity = entity.clone();
        let selected_root =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .map(|session| session.id.clone());
        let live_root =
            root_session_for_path(&self.sessions, self.snapshot.live_session.as_deref())
                .map(|session| session.id.clone());
        let (rows, project_count) = session_rail_items(&self.sessions);
        let row_count = rows.len();
        let row_entity = entity.clone();
        let live_status = self.snapshot.live_status.clone();
        let session_list = uniform_list("session-list", row_count, move |range, _, _| {
            range
                .filter_map(|index| rows.get(index))
                .map(|item| {
                    let selected = selected_root.as_deref() == Some(item.session.id.as_str());
                    let badge = session_badge(
                        item.kind,
                        &item.session.id,
                        live_root.as_deref(),
                        &live_status,
                    );
                    session_row(item, selected, badge, row_entity.clone())
                })
                .collect::<Vec<_>>()
        })
        .size_full();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(THEME.colors.panel)
            .child(
                div()
                    .h(THEME.layout.header_height)
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(THEME.space.md)
                    .child(
                        div()
                            .text_size(THEME.type_scale.heading)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(THEME.colors.text)
                            .child("Pi"),
                    )
                    .child(icon_button(
                        "new-session",
                        AppIcon::Plus,
                        "New session",
                        ButtonTone::Quiet,
                        self.snapshot.connected,
                        move |_, cx| {
                            let _ = new_entity.update(cx, |this, cx| this.new_session(cx));
                        },
                    )),
            )
            .child(
                div()
                    .mx(THEME.space.sm)
                    .mb(THEME.space.xs)
                    .h(px(40.0))
                    .flex()
                    .items_center()
                    .gap(THEME.space.sm)
                    .px(THEME.space.sm)
                    .rounded(THEME.radius)
                    .bg(THEME.colors.surface)
                    .text_color(THEME.colors.muted)
                    .child(Icon::new(AppIcon::MagnifyingGlass).with_size(Size::Small))
                    .child(
                        Input::new(&self.search)
                            .flex_1()
                            .min_w_0()
                            .appearance(false),
                    ),
            )
            .child(
                div()
                    .h(px(38.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(THEME.space.sm)
                    .px(THEME.space.md)
                    .text_color(THEME.colors.muted)
                    .child(Icon::new(AppIcon::Folder).with_size(Size::Small))
                    .child(
                        div()
                            .flex_1()
                            .text_size(THEME.type_scale.body)
                            .font_weight(FontWeight::MEDIUM)
                            .child("All projects"),
                    )
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(project_count.to_string()),
                    ),
            )
            .when_some(self.sessions_error.clone(), |rail, error| {
                rail.child(feedback("sessions-error", error, FeedbackTone::Error))
            })
            .child(
                div()
                    .id("session-list-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_hidden()
                    .child(session_list),
            )
            .when(row_count == 0 && self.sessions_error.is_none(), |rail| {
                rail.child(
                    div()
                        .px(THEME.space.md)
                        .py(THEME.space.sm)
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child("No matching sessions"),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_run_panel(&self, entity: WeakEntity<Self>) -> impl IntoElement {
        let retry_entity = entity.clone();
        let compact_entity = entity.clone();
        let abort_retry_entity = entity.clone();
        let auto_retry_entity = entity.clone();
        let auto_compact_entity = entity.clone();
        let queue = &self.snapshot.conversation.queue;
        let has_queue = !queue.steering.is_empty() || !queue.follow_up.is_empty();
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
            agent_rows.push((
                root.clone(),
                0,
                "Main".into(),
                self.snapshot.selected_session.as_deref() == Some(root.path.as_path()),
            ));
            agent_rows.extend(descendants.into_iter().map(|(session, depth)| {
                (
                    session.clone(),
                    depth,
                    compact_subagent_label(&session.title),
                    self.snapshot.selected_session.as_deref() == Some(session.path.as_path()),
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
                .map(|(session, depth, label, selected)| {
                    agent_session_row(
                        session,
                        *depth,
                        label.clone(),
                        *selected,
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
            .child(section_heading("Status"))
            .when_some(self.fps_monitor.clone(), |run, monitor| run.child(monitor))
            .child(usage_metrics(
                show_main_context.then_some(&self.snapshot.stats),
                aggregate_usage,
            ))
            .when(!self.snapshot.history_preview, |run| {
                run.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(THEME.space.xs)
                        .child(icon_button(
                            "compact",
                            AppIcon::Archive,
                            "Compact context",
                            ButtonTone::Neutral,
                            self.snapshot.connected && !self.snapshot.conversation.running,
                            move |_, cx| {
                                let _ = compact_entity
                                    .update(cx, |this, _| this.send(RuntimeCommand::Compact));
                            },
                        ))
                        .when(!self.snapshot.connected, |actions| {
                            actions.child(icon_button(
                                "restart",
                                AppIcon::ArrowClockwise,
                                "Reconnect",
                                ButtonTone::Neutral,
                                true,
                                move |_, cx| {
                                    let _ = retry_entity
                                        .update(cx, |this, _| this.send(RuntimeCommand::Restart));
                                },
                            ))
                        })
                        .when(self.snapshot.conversation.retrying, |actions| {
                            actions.child(icon_button(
                                "abort-retry",
                                AppIcon::Stop,
                                "Stop retry",
                                ButtonTone::Danger,
                                true,
                                move |_, cx| {
                                    let _ = abort_retry_entity.update(cx, |this, _| {
                                        this.send(RuntimeCommand::AbortRetry)
                                    });
                                },
                            ))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(THEME.space.xs)
                        .child(button(
                            "auto-retry",
                            format!("Retry {}", on_off(self.snapshot.auto_retry)),
                            ButtonTone::Quiet,
                            self.snapshot.connected,
                            move |_, cx| {
                                let _ = auto_retry_entity.update(cx, |this, _| {
                                    this.send(RuntimeCommand::SetAutoRetry(
                                        !this.snapshot.auto_retry,
                                    ))
                                });
                            },
                        ))
                        .child(button(
                            "auto-compact",
                            format!(
                                "Compact {}",
                                on_off(
                                    self.snapshot
                                        .session
                                        .as_ref()
                                        .is_some_and(|state| state.auto_compaction_enabled)
                                )
                            ),
                            ButtonTone::Quiet,
                            self.snapshot.connected,
                            move |_, cx| {
                                let _ = auto_compact_entity.update(cx, |this, _| {
                                    let enabled = this
                                        .snapshot
                                        .session
                                        .as_ref()
                                        .is_some_and(|state| state.auto_compaction_enabled);
                                    this.send(RuntimeCommand::SetAutoCompaction(!enabled));
                                });
                            },
                        )),
                )
            })
            .when(agent_count > 0, |run| {
                run.child(section_heading("Agents"))
                    .child(div().overflow_y_hidden().child(agent_list))
            })
            .when(has_queue, |run| {
                run.child(section_heading("Queue")).child(
                    div()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.muted)
                        .child(format!(
                            "{} steer · {} later",
                            queue.steering.len(),
                            queue.follow_up.len()
                        )),
                )
            })
            .when(!self.extension_errors.is_empty(), |run| {
                run.child(section_heading("Extension errors")).children(
                    self.extension_errors
                        .iter()
                        .enumerate()
                        .map(|(index, error)| {
                            feedback(
                                ("extension-error", index),
                                error.clone(),
                                FeedbackTone::Error,
                            )
                        }),
                )
            })
            .when(!self.snapshot.stderr.is_empty(), |run| {
                run.child(feedback(
                    "stderr",
                    self.snapshot.stderr.clone(),
                    FeedbackTone::Warning,
                ))
            });
        panel()
            .size_full()
            .rounded_none()
            .border_0()
            .child(
                div()
                    .h(THEME.layout.header_height)
                    .flex_none()
                    .flex()
                    .items_center()
                    .px(THEME.space.sm)
                    .border_b(THEME.border)
                    .border_color(THEME.colors.border)
                    .text_size(THEME.type_scale.caption)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(THEME.colors.muted)
                    .child("RUN"),
            )
            .child(body)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionRailKind {
    Project,
    Settled,
}

#[derive(Clone, Debug)]
struct SessionRailItem {
    session: SessionSummary,
    kind: SessionRailKind,
    starts_settled: bool,
}

fn session_rail_items(sessions: &[SessionSummary]) -> (Vec<SessionRailItem>, usize) {
    let mut projects = HashSet::new();
    let mut current = Vec::new();
    let mut settled = Vec::new();
    for session in root_sessions(sessions) {
        if projects.insert(session.project.clone()) {
            current.push(SessionRailItem {
                session: session.clone(),
                kind: SessionRailKind::Project,
                starts_settled: false,
            });
        } else {
            settled.push(SessionRailItem {
                session: session.clone(),
                kind: SessionRailKind::Settled,
                starts_settled: false,
            });
        }
    }
    if let Some(first) = settled.first_mut() {
        first.starts_settled = true;
    }
    let project_count = current.len();
    current.extend(settled);
    (current, project_count)
}

fn session_badge(
    kind: SessionRailKind,
    session_id: &str,
    live_session_id: Option<&str>,
    live_status: &str,
) -> Option<String> {
    if live_session_id == Some(session_id) {
        Some(if live_status.is_empty() {
            "Done".into()
        } else {
            live_status.into()
        })
    } else if kind == SessionRailKind::Project {
        Some("Done".into())
    } else {
        None
    }
}

fn session_row(
    item: &SessionRailItem,
    selected: bool,
    status: Option<String>,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let session = &item.session;
    let path = session.path.clone();
    let project = session.project.clone();
    let project_name = project_label(&project);
    let age = relative_age(session.modified);
    let is_settled = item.kind == SessionRailKind::Settled;
    let metadata = if item.starts_settled {
        format!("Settled · {project_name}")
    } else {
        project_name
    };
    let icon = if is_settled {
        AppIcon::ChatCircle
    } else {
        AppIcon::Folder
    };
    let row = div()
        .id(format!("session-{}", session.id))
        .role(Role::Button)
        .aria_label(format!("Resume session: {}", session.title))
        .aria_selected(selected)
        .tab_index(0)
        .size_full()
        .h(THEME.layout.session_row_height)
        .flex()
        .flex_col()
        .justify_center()
        .gap(THEME.space.xs)
        .px(THEME.space.sm)
        .rounded(THEME.radius)
        .bg(if selected {
            THEME.colors.surface
        } else {
            THEME.colors.panel
        })
        .hover(|row| row.bg(THEME.colors.hover))
        .focus(|row| row.border(THEME.border).border_color(THEME.colors.accent))
        .cursor(CursorStyle::PointingHand)
        .on_click(move |_, _, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.resume(path.clone(), project.clone(), cx)
            });
        })
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap(THEME.space.xs)
                .text_color(if is_settled {
                    THEME.colors.subtle
                } else {
                    THEME.colors.muted
                })
                .child(Icon::new(icon).with_size(Size::Small))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(THEME.type_scale.caption)
                        .font_weight(if item.starts_settled {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .child(metadata),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(THEME.type_scale.caption)
                        .text_color(match status.as_deref() {
                            Some("Done") => THEME.colors.success,
                            Some(_) => THEME.colors.accent,
                            None => THEME.colors.subtle,
                        })
                        .child(status.unwrap_or(age)),
                ),
        )
        .child(
            div()
                .w_full()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(THEME.type_scale.body)
                .font_weight(if selected || !is_settled {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if is_settled && !selected {
                    THEME.colors.muted
                } else {
                    THEME.colors.text
                })
                .child(session.title.clone()),
        );
    div()
        .h(THEME.layout.session_row_height)
        .w_full()
        .px(THEME.space.sm)
        .child(row)
        .into_any_element()
}

fn project_label(project: &Path) -> String {
    project
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| project.display().to_string(), str::to_owned)
}

fn relative_age(modified: SystemTime) -> String {
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO);
    if age < Duration::from_secs(60) {
        "now".into()
    } else if age < Duration::from_secs(60 * 60) {
        format!("{}m", age.as_secs() / 60)
    } else if age < Duration::from_secs(24 * 60 * 60) {
        format!("{}h", age.as_secs() / (60 * 60))
    } else {
        format!("{}d", age.as_secs() / (24 * 60 * 60))
    }
}

fn agent_session_row(
    session: &SessionSummary,
    depth: usize,
    label: String,
    selected: bool,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let path = session.path.clone();
    let project = session.project.clone();
    let title = session.title.clone();
    let tokens = compact_number(session.usage.total);
    div()
        .id(format!("agent-session-{}", session.id))
        .role(Role::Button)
        .aria_label(format!("Open agent session: {title}"))
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
        .on_click(move |_, _, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.resume(path.clone(), project.clone(), cx)
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
                .text_color(THEME.colors.subtle)
                .child(tokens),
        )
        .into_any_element()
}

fn usage_metrics(stats: Option<&serde_json::Value>, usage: UsageSummary) -> impl IntoElement {
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

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

#[cfg(test)]
mod tests {
    use super::{
        SessionRailKind, compact_number, compact_subagent_label, context_usage, format_cost,
        session_badge,
    };

    #[test]
    fn subagent_labels_keep_the_role_and_drop_generated_ids() {
        assert_eq!(
            compact_subagent_label("subagent-reviewer-a7d59830-87da-46d7-1"),
            "reviewer 1"
        );
        assert_eq!(compact_subagent_label("named child"), "named child");
    }

    #[test]
    fn usage_values_are_compact_and_context_is_main_only() {
        assert_eq!(compact_number(105_250), "105.2k");
        assert_eq!(format_cost(456_789), "$0.46");
        assert_eq!(
            context_usage(&serde_json::json!({
                "contextUsage": {"tokens": 60_000, "contextWindow": 200_000, "percent": 30.0}
            })),
            "60k / 200k · 30%"
        );
    }

    #[test]
    fn session_badges_do_not_depend_on_selection() {
        assert_eq!(
            session_badge(SessionRailKind::Project, "other", Some("live"), "Working"),
            Some("Done".into())
        );
        assert_eq!(
            session_badge(SessionRailKind::Settled, "live", Some("live"), "Working"),
            Some("Working".into())
        );
        assert_eq!(
            session_badge(SessionRailKind::Settled, "old", Some("live"), "Working"),
            None
        );
    }
}
