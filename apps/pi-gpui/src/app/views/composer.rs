use gpui::{
    FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _,
};
use gpui_component::{FocusTrapElement as _, input::Input};

use super::super::PiApp;
use crate::{
    primitives::{ButtonTone, button, dialog_backdrop, dialog_surface},
    protocol::{ExtensionUiRequest, PromptMode},
    runtime::RuntimeCommand,
    theme::THEME,
};

impl PiApp {
    pub(super) fn render_composer(&self, entity: WeakEntity<Self>) -> impl IntoElement {
        let widgets_above = widget_region("above", &self.extension.above_widgets);
        let widgets_below = widget_region("below", &self.extension.below_widgets);
        let normal = entity.clone();
        let steer = entity.clone();
        let follow = entity.clone();
        let send_entity = entity.clone();
        let abort_entity = entity;
        div()
            .flex_none()
            .min_h(THEME.layout.composer_min)
            .border_t(THEME.border)
            .border_color(THEME.colors.border)
            .bg(THEME.colors.panel)
            .p(THEME.space.sm)
            .when_some(widgets_above, |composer, widgets| composer.child(widgets))
            .child(
                div()
                    .px(THEME.space.sm)
                    .child(Input::new(&self.composer).w_full().appearance(false)),
            )
            .when_some(widgets_below, |composer, widgets| composer.child(widgets))
            .child(
                div()
                    .mt(THEME.space.sm)
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(THEME.space.sm)
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(THEME.space.xs)
                            .child(button(
                                "mode-normal",
                                "Prompt",
                                tone(self.prompt_mode == PromptMode::Normal),
                                true,
                                move |_, cx| {
                                    let _ = normal.update(cx, |this, cx| {
                                        this.set_prompt_mode(PromptMode::Normal, cx)
                                    });
                                },
                            ))
                            .child(button(
                                "mode-steer",
                                "Steer",
                                tone(self.prompt_mode == PromptMode::Steer),
                                !self.snapshot.history_preview,
                                move |_, cx| {
                                    let _ = steer.update(cx, |this, cx| {
                                        this.set_prompt_mode(PromptMode::Steer, cx)
                                    });
                                },
                            ))
                            .child(button(
                                "mode-follow",
                                "Follow-up",
                                tone(self.prompt_mode == PromptMode::FollowUp),
                                !self.snapshot.history_preview,
                                move |_, cx| {
                                    let _ = follow.update(cx, |this, cx| {
                                        this.set_prompt_mode(PromptMode::FollowUp, cx)
                                    });
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(THEME.space.xs)
                            .when(self.snapshot.conversation.running, |actions| {
                                actions.child(button(
                                    "abort",
                                    "Abort",
                                    ButtonTone::Danger,
                                    true,
                                    move |_, cx| {
                                        let _ = abort_entity
                                            .update(cx, |this, _| this.send(RuntimeCommand::Abort));
                                    },
                                ))
                            })
                            .child(button(
                                "send",
                                "Send",
                                ButtonTone::Accent,
                                self.can_submit(),
                                move |_window, cx| {
                                    let _ = send_entity.update(cx, |this, cx| {
                                        let value =
                                            this.composer.read(cx).value().trim().to_owned();
                                        if !value.is_empty() {
                                            this.submit(value, cx);
                                        }
                                    });
                                },
                            )),
                    ),
            )
    }

    pub(super) fn render_dialog(&self, entity: WeakEntity<Self>) -> Option<impl IntoElement> {
        let dialog = self.extension.dialog.as_ref()?;
        let id = dialog.dialog_id()?.to_owned();
        let cancel_entity = entity.clone();
        let cancel_button_entity = entity.clone();
        let (title, body) = match dialog {
            ExtensionUiRequest::Select { title, options, .. } => {
                let buttons = options
                    .iter()
                    .enumerate()
                    .map(|(index, option)| {
                        let value = option.clone();
                        let id = id.clone();
                        let choice_entity = entity.clone();
                        button(
                            ("dialog-option", index),
                            option.clone(),
                            ButtonTone::Neutral,
                            true,
                            move |window, cx| {
                                let _ = choice_entity.update(cx, |this, cx| {
                                    if let Some(response) =
                                        this.extension.respond_value(&id, value.clone())
                                    {
                                        this.send(RuntimeCommand::ExtensionResponse(response));
                                        this.advance_or_restore_dialog(window, cx);
                                    }
                                });
                            },
                        )
                    })
                    .collect::<Vec<_>>();
                (
                    title.clone(),
                    div()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.xs)
                        .children(buttons)
                        .into_any_element(),
                )
            }
            ExtensionUiRequest::Confirm { title, message, .. } => {
                let yes_id = id.clone();
                let no_id = id.clone();
                let yes = entity.clone();
                let no = entity.clone();
                (
                    title.clone(),
                    div()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.md)
                        .child(div().child(message.clone()))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap(THEME.space.xs)
                                .child(button(
                                    "confirm-no",
                                    "No",
                                    ButtonTone::Neutral,
                                    true,
                                    move |window, cx| {
                                        let _ = no.update(cx, |this, cx| {
                                            this.respond_confirm(no_id.clone(), false, window, cx)
                                        });
                                    },
                                ))
                                .child(button(
                                    "confirm-yes",
                                    "Yes",
                                    ButtonTone::Accent,
                                    true,
                                    move |window, cx| {
                                        let _ = yes.update(cx, |this, cx| {
                                            this.respond_confirm(yes_id.clone(), true, window, cx)
                                        });
                                    },
                                )),
                        )
                        .into_any_element(),
                )
            }
            ExtensionUiRequest::Input {
                title, placeholder, ..
            }
            | ExtensionUiRequest::Editor {
                title,
                prefill: placeholder,
                ..
            } => {
                let submit_id = id.clone();
                let submit = entity.clone();
                (
                    title.clone(),
                    div()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.md)
                        .when_some(placeholder.clone(), |body, hint| {
                            body.child(
                                div()
                                    .text_size(THEME.type_scale.caption)
                                    .text_color(THEME.colors.subtle)
                                    .child(hint),
                            )
                        })
                        .child(Input::new(&self.dialog_input).w_full())
                        .child(div().flex().justify_end().child(button(
                            "dialog-submit",
                            "Continue",
                            ButtonTone::Accent,
                            true,
                            move |window, cx| {
                                let _ = submit.update(cx, |this, cx| {
                                    this.respond_value(submit_id.clone(), window, cx)
                                });
                            },
                        )))
                        .into_any_element(),
                )
            }
            _ => return None,
        };
        Some(
            dialog_backdrop("extension-dialog-backdrop", move |window, cx| {
                let _ = cancel_entity.update(cx, |this, cx| this.cancel_dialog(window, cx));
            })
            .child(
                dialog_surface("extension-dialog", title.clone())
                    .track_focus(&self.dialog_focus)
                    .key_context(super::OVERLAY_KEY_CONTEXT)
                    .max_w_full()
                    .p(THEME.space.md)
                    .child(
                        div()
                            .mb(THEME.space.md)
                            .text_size(THEME.type_scale.heading)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(body)
                    .child(div().mt(THEME.space.md).flex().justify_end().child(button(
                        "dialog-cancel",
                        "Cancel",
                        ButtonTone::Quiet,
                        true,
                        move |window, cx| {
                            let _ = cancel_button_entity
                                .update(cx, |this, cx| this.cancel_dialog(window, cx));
                        },
                    )))
                    .focus_trap("extension-dialog-trap", &self.dialog_focus),
            ),
        )
    }
}

fn tone(active: bool) -> ButtonTone {
    if active {
        ButtonTone::Accent
    } else {
        ButtonTone::Quiet
    }
}

fn widget_region(
    placement: &str,
    widgets: &std::collections::BTreeMap<String, Vec<String>>,
) -> Option<gpui::AnyElement> {
    if widgets.is_empty() {
        return None;
    }
    Some(
        div()
            .id(format!("widgets-{placement}"))
            .max_h(THEME.layout.tool_max_height)
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(THEME.space.xs)
            .children(widgets.iter().map(|(key, lines)| {
                div()
                    .id(format!("widget-{placement}-{key}"))
                    .px(THEME.space.sm)
                    .py(THEME.space.xs)
                    .bg(THEME.colors.surface)
                    .font_family("monospace")
                    .text_size(THEME.type_scale.caption)
                    .children(lines.iter().cloned().map(|line| div().child(line)))
            }))
            .into_any_element(),
    )
}
