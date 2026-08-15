use gpui::{
    AnyElement, CursorStyle, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Role, StatefulInteractiveElement as _, Styled as _, WeakEntity, accesskit, div,
    prelude::FluentBuilder as _, px, uniform_list,
};
use gpui_component::input::Input;

use super::super::{MAX_SESSION_ROWS, PiApp};
use crate::{
    assets::AppIcon,
    layout::LayoutMode,
    primitives::{ButtonTone, FeedbackTone, button, feedback, icon_button, panel, section_heading},
    runtime::RuntimeCommand,
    sessions::{SessionSummary, descendant_sessions, root_session_for_path, root_sessions},
    theme::THEME,
};

impl PiApp {
    pub(super) fn render_header(
        &self,
        mode: LayoutMode,
        entity: WeakEntity<Self>,
    ) -> impl IntoElement {
        let project = self
            .project
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
        let roots = root_sessions(&self.sessions);
        let selected_root =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .map(|session| session.id.clone());
        let rows = roots
            .iter()
            .take(MAX_SESSION_ROWS)
            .map(|session| (*session).clone())
            .collect::<Vec<_>>();
        let row_count = rows.len();
        let row_entity = entity.clone();
        let session_list = uniform_list("session-list", row_count, move |range, _, _| {
            range
                .filter_map(|index| rows.get(index))
                .map(|session| {
                    session_row(
                        session,
                        selected_root.as_deref() == Some(session.id.as_str()),
                        row_entity.clone(),
                    )
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
                    .px(THEME.space.sm)
                    .border_b(THEME.border)
                    .border_color(THEME.colors.border)
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(THEME.colors.muted)
                            .child("SESSIONS"),
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
                    .p(THEME.space.sm)
                    .border_b(THEME.border)
                    .border_color(THEME.colors.border)
                    .child(Input::new(&self.search).w_full().appearance(false)),
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
            .when(roots.len() > MAX_SESSION_ROWS, |rail| {
                rail.child(
                    div()
                        .p(THEME.space.sm)
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child("Refine the search to see more sessions"),
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
        let mut agent_rows = Vec::new();
        if !descendants.is_empty()
            && let Some(root) = root
        {
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
        let statuses = self
            .extension
            .statuses
            .iter()
            .map(|(key, value)| {
                div()
                    .h(THEME.layout.status_row_height)
                    .flex()
                    .items_center()
                    .gap(THEME.space.sm)
                    .border_b(THEME.border)
                    .border_color(THEME.colors.border)
                    .text_size(THEME.type_scale.caption)
                    .child(
                        div()
                            .w(THEME.layout.status_key_width)
                            .flex_none()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(THEME.colors.subtle)
                            .child(key.clone()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(THEME.colors.muted)
                            .child(strip_terminal_control(value)),
                    )
            })
            .collect::<Vec<_>>();
        let body = div()
            .id("run-panel-scroll")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .p(THEME.space.sm)
            .gap(THEME.space.md)
            .overflow_y_scroll()
            .when(agent_count > 0, |run| {
                run.child(div().overflow_y_hidden().child(agent_list))
            })
            .when(!self.snapshot.history_preview, |run| {
                run.child(section_heading("Session"))
                    .child(
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
                                        let _ = retry_entity.update(cx, |this, _| {
                                            this.send(RuntimeCommand::Restart)
                                        });
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
            .when(!statuses.is_empty(), |run| {
                run.child(section_heading("Status")).children(statuses)
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
                    .child(if agent_count > 0 { "AGENTS" } else { "RUN" }),
            )
            .child(body)
    }
}

fn session_row(session: &SessionSummary, selected: bool, entity: WeakEntity<PiApp>) -> AnyElement {
    let path = session.path.clone();
    div()
        .id(format!("session-{}", session.id))
        .role(Role::Button)
        .aria_label(format!("Resume session: {}", session.title))
        .aria_selected(selected)
        .tab_index(0)
        .h(THEME.layout.session_row_height)
        .w_full()
        .flex()
        .items_center()
        .gap(THEME.space.sm)
        .px(THEME.space.sm)
        .border_b(THEME.border)
        .border_color(THEME.colors.border)
        .bg(if selected {
            THEME.colors.surface
        } else {
            THEME.colors.panel
        })
        .when(selected, |row| {
            row.border_l(px(2.0)).border_color(THEME.colors.accent)
        })
        .hover(|row| row.bg(THEME.colors.hover))
        .focus(|row| row.border(THEME.border).border_color(THEME.colors.accent))
        .cursor(CursorStyle::PointingHand)
        .on_click(move |_, _, cx| {
            let _ = entity.update(cx, |this, cx| this.resume(path.clone(), cx));
        })
        .child(
            div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(THEME.type_scale.body)
                .text_color(THEME.colors.text)
                .child(session.title.clone()),
        )
        .child(
            div()
                .flex_none()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child(session.message_count.to_string()),
        )
        .into_any_element()
}

fn agent_session_row(
    session: &SessionSummary,
    depth: usize,
    label: String,
    selected: bool,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let path = session.path.clone();
    let title = session.title.clone();
    let message_count = session.message_count;
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
            let _ = entity.update(cx, |this, cx| this.resume(path.clone(), cx));
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
                .child(message_count.to_string()),
        )
        .into_any_element()
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

fn strip_terminal_control(value: &str) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    let mut clean = String::with_capacity(value.len());
    let mut index = 0;
    while index < chars.len() {
        let starts_escape = chars[index] == '\u{1b}' && chars.get(index + 1) == Some(&'[');
        let starts_bare_sgr = chars[index] == '[';
        if starts_escape || starts_bare_sgr {
            let start = index + usize::from(starts_escape) + 1;
            let mut end = start;
            while chars
                .get(end)
                .is_some_and(|character| character.is_ascii_digit() || *character == ';')
            {
                end += 1;
            }
            if end > start && chars.get(end) == Some(&'m') {
                index = end + 1;
                continue;
            }
        }
        clean.push(chars[index]);
        index += 1;
    }
    clean
}

#[cfg(test)]
mod tests {
    use super::{compact_subagent_label, strip_terminal_control};

    #[test]
    fn subagent_labels_keep_the_role_and_drop_generated_ids() {
        assert_eq!(
            compact_subagent_label("subagent-reviewer-a7d59830-87da-46d7-1"),
            "reviewer 1"
        );
        assert_eq!(compact_subagent_label("named child"), "named child");
    }

    #[test]
    fn terminal_colors_never_leak_into_status_copy() {
        assert_eq!(
            strip_terminal_control("sandbox: \u{1b}[38;2;142;192;124m🔒 Codex\u{1b}[39m"),
            "sandbox: 🔒 Codex"
        );
        assert_eq!(
            strip_terminal_control("IO: [39mpi-sandbox[39m"),
            "IO: pi-sandbox"
        );
    }
}
