use gpui::{
    FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, accesskit, div,
    prelude::FluentBuilder as _,
};
use gpui_component::input::Input;

use super::super::{MAX_SESSION_ROWS, PiApp};
use crate::{
    layout::LayoutMode,
    primitives::{ButtonTone, FeedbackTone, button, feedback, panel, section_heading},
    runtime::RuntimeCommand,
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
        let status_accessible = self.snapshot.status.clone();
        let model_entity = entity.clone();
        let thinking_entity = entity.clone();
        let sessions_entity = entity.clone();
        let run_entity = entity.clone();
        div()
            .min_h(THEME.layout.header_height)
            .flex_none()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(THEME.space.md)
            .px(THEME.space.md)
            .py(THEME.space.xs)
            .border_b(THEME.border)
            .border_color(THEME.colors.border)
            .bg(THEME.colors.panel)
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(THEME.space.sm)
                    .child(
                        div()
                            .text_size(THEME.type_scale.heading)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Pi"),
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
                    .flex_wrap()
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
                    .child(button(
                        "cycle-model",
                        model,
                        ButtonTone::Neutral,
                        !self.snapshot.models.is_empty(),
                        move |_, cx| {
                            let _ = model_entity.update(cx, |this, cx| this.cycle_model(cx));
                        },
                    ))
                    .child(button(
                        "cycle-thinking",
                        thinking,
                        ButtonTone::Neutral,
                        !self.snapshot.thinking_levels.is_empty(),
                        move |_, cx| {
                            let _ = thinking_entity.update(cx, |this, cx| this.cycle_thinking(cx));
                        },
                    ))
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
                            .text_size(THEME.type_scale.caption)
                            .text_color(if self.snapshot.connected {
                                THEME.colors.accent
                            } else {
                                THEME.colors.error
                            })
                            .child(self.snapshot.status.clone()),
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
        let rows = self
            .sessions
            .iter()
            .take(MAX_SESSION_ROWS)
            .map(|session| {
                let path = session.path.clone();
                let element_id = format!("session-{}", session.id);
                let row_entity = entity.clone();
                let selected = self.snapshot.selected_session.as_deref() == Some(path.as_path());
                div()
                    .id(element_id)
                    .role(Role::Button)
                    .aria_label(format!("Resume session: {}", session.title))
                    .aria_selected(selected)
                    .tab_index(0)
                    .px(THEME.space.sm)
                    .py(THEME.space.sm)
                    .border_b(THEME.border)
                    .border_color(THEME.colors.border)
                    .when(selected, |row| row.bg(THEME.colors.surface))
                    .hover(|row| row.bg(THEME.colors.hover))
                    .focus(|row| row.border(THEME.border).border_color(THEME.colors.accent))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        let _ = row_entity.update(cx, |this, cx| this.resume(path.clone(), cx));
                    })
                    .child(
                        div()
                            .text_size(THEME.type_scale.body)
                            .text_color(THEME.colors.text)
                            .child(session.title.clone()),
                    )
                    .child(
                        div()
                            .mt(THEME.space.xs)
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(format!("{} messages", session.message_count)),
                    )
            })
            .collect::<Vec<_>>();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(THEME.colors.panel)
            .child(
                div()
                    .p(THEME.space.sm)
                    .border_b(THEME.border)
                    .border_color(THEME.colors.border)
                    .child(Input::new(&self.search).w_full()),
            )
            .child(div().p(THEME.space.sm).child(button(
                "new-session",
                "New session",
                ButtonTone::Accent,
                self.snapshot.connected,
                move |_, cx| {
                    let _ = new_entity.update(cx, |this, cx| this.new_session(cx));
                },
            )))
            .when_some(self.sessions_error.clone(), |rail, error| {
                rail.child(feedback("sessions-error", error, FeedbackTone::Error))
            })
            .child(
                div()
                    .id("session-list-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .children(rows),
            )
            .when(self.sessions.len() > MAX_SESSION_ROWS, |rail| {
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
        let auto_compact_entity = entity;
        let queue = &self.snapshot.conversation.queue;
        let statuses = self
            .extension
            .statuses
            .iter()
            .map(|(key, value)| {
                div()
                    .text_size(THEME.type_scale.caption)
                    .child(format!("{key}: {value}"))
            })
            .collect::<Vec<_>>();
        panel()
            .id("run-panel-scroll")
            .size_full()
            .rounded_none()
            .border_0()
            .p(THEME.space.md)
            .gap(THEME.space.md)
            .overflow_y_scroll()
            .child(section_heading("Run"))
            .child(
                div()
                    .text_size(THEME.type_scale.body)
                    .child(self.snapshot.status.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(THEME.space.xs)
                    .child(button(
                        "compact",
                        "Compact",
                        ButtonTone::Neutral,
                        self.snapshot.connected && !self.snapshot.conversation.running,
                        move |_, cx| {
                            let _ = compact_entity
                                .update(cx, |this, _| this.send(RuntimeCommand::Compact));
                        },
                    ))
                    .child(button(
                        "restart",
                        "Reconnect",
                        ButtonTone::Neutral,
                        !self.snapshot.connected,
                        move |_, cx| {
                            let _ = retry_entity
                                .update(cx, |this, _| this.send(RuntimeCommand::Restart));
                        },
                    ))
                    .child(button(
                        "abort-retry",
                        "Stop retry",
                        ButtonTone::Danger,
                        self.snapshot.conversation.retrying,
                        move |_, cx| {
                            let _ = abort_retry_entity
                                .update(cx, |this, _| this.send(RuntimeCommand::AbortRetry));
                        },
                    )),
            )
            .child(button(
                "auto-retry",
                format!("Auto retry: {}", on_off(self.snapshot.auto_retry)),
                ButtonTone::Quiet,
                self.snapshot.connected,
                move |_, cx| {
                    let _ = auto_retry_entity.update(cx, |this, _| {
                        this.send(RuntimeCommand::SetAutoRetry(!this.snapshot.auto_retry))
                    });
                },
            ))
            .child(button(
                "auto-compact",
                format!(
                    "Auto compact: {}",
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
            ))
            .child(section_heading("Queue"))
            .child(
                div()
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.muted)
                    .child(format!(
                        "{} steering · {} follow-up",
                        queue.steering.len(),
                        queue.follow_up.len()
                    )),
            )
            .child(section_heading("Extension status"))
            .when(statuses.is_empty(), |run| {
                run.child(
                    div()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child("No extension status"),
                )
            })
            .children(statuses)
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
            })
    }
}

fn bounded_label(value: &str, max: usize) -> String {
    let mut label = value.chars().take(max).collect::<String>();
    if value.chars().count() > max {
        label.push('…');
    }
    label
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}
