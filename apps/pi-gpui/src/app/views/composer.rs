use std::rc::Rc;

use gpui::{
    AnyElement, App, CursorStyle, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::input::Input;

use super::super::PiApp;
use crate::{
    primitives::{ButtonTone, button},
    protocol::ExtensionUiRequest,
    runtime::RuntimeCommand,
    theme::THEME,
};

impl PiApp {
    pub(super) fn render_composer(&self, entity: WeakEntity<Self>) -> AnyElement {
        if self.extension.dialog.is_some() {
            return self.render_composer_request(entity);
        }
        let widgets_above = widget_region("above", &self.extension.above_widgets);
        let widgets_below = widget_region("below", &self.extension.below_widgets);
        let send_entity = entity.clone();
        let history_entity = entity.clone();
        let paste_entity = entity.clone();
        let cursor_entity = entity.clone();
        let attachments_entity = entity.clone();
        let abort_entity = entity;
        div()
            .flex_none()
            .min_h(THEME.layout.composer_min)
            .border_t(THEME.border)
            .border_color(THEME.colors.border)
            .bg(THEME.colors.panel)
            .p(THEME.space.sm)
            .when_some(widgets_above, |composer, widgets| composer.child(widgets))
            .when_some(composer_status(self), |composer, status| {
                composer.child(status)
            })
            .when_some(
                super::attachments::render(self, attachments_entity),
                |composer, attachments| composer.child(attachments),
            )
            .child(
                div()
                    .id("composer-input")
                    .key_context(super::super::COMPOSER_KEY_CONTEXT)
                    .px(THEME.space.sm)
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "v"
                            && event.keystroke.modifiers.secondary()
                            && paste_entity
                                .update(cx, |this, cx| this.paste_composer_image(cx))
                                .unwrap_or(false)
                        {
                            window.prevent_default();
                            cx.stop_propagation();
                            return;
                        }
                        let handled = history_entity
                            .update(cx, |this, cx| {
                                this.handle_composer_history_key(
                                    event.keystroke.key.as_str(),
                                    window,
                                    cx,
                                )
                            })
                            .unwrap_or(false);
                        if handled {
                            window.prevent_default();
                        } else {
                            capture_after_input(history_entity.clone(), cx);
                        }
                    })
                    .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                        capture_after_input(cursor_entity.clone(), cx);
                    })
                    .child(Input::new(&self.composer).w_full().appearance(false)),
            )
            .when_some(widgets_below, |composer, widgets| composer.child(widgets))
            .child(
                div()
                    .mt(THEME.space.sm)
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_end()
                    .gap(THEME.space.sm)
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
                                if self.snapshot.conversation.running {
                                    "Steer"
                                } else {
                                    "Send"
                                },
                                ButtonTone::Accent,
                                self.can_submit(),
                                move |_window, cx| {
                                    let _ = send_entity.update(cx, |this, cx| {
                                        let value =
                                            this.composer.read(cx).value().trim().to_owned();
                                        if !value.is_empty() || this.has_composer_images() {
                                            this.submit(value, this.enter_mode(), cx);
                                        }
                                    });
                                },
                            )),
                    ),
            )
            .into_any_element()
    }

    fn render_composer_request(&self, entity: WeakEntity<Self>) -> AnyElement {
        let Some(dialog) = self.extension.dialog.as_ref() else {
            return div().into_any_element();
        };
        let Some(id) = dialog.dialog_id().map(str::to_owned) else {
            return div().into_any_element();
        };
        let cancel_button_entity = entity.clone();
        let (title, body) = match dialog {
            ExtensionUiRequest::Select { title, options, .. } => {
                let choices = options
                    .iter()
                    .enumerate()
                    .map(|(index, option)| {
                        let value = option.clone();
                        let id = id.clone();
                        let choice_entity = entity.clone();
                        dialog_choice(
                            ("dialog-option", index),
                            option,
                            index == 0,
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
                let (heading, prompt) = dialog_copy(title);
                (
                    heading,
                    div()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.md)
                        .when_some(prompt, |body, prompt| {
                            body.child(
                                div()
                                    .text_size(THEME.type_scale.body)
                                    .text_color(THEME.colors.muted)
                                    .line_height(px(22.0))
                                    .child(prompt),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(THEME.space.xs)
                                .children(choices),
                        )
                        .into_any_element(),
                )
            }
            ExtensionUiRequest::Confirm { title, message, .. } => {
                let yes_id = id.clone();
                let no_id = id.clone();
                let yes = entity.clone();
                let no = entity.clone();
                (
                    SharedString::from(title.clone()),
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
                    SharedString::from(title.clone()),
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
            _ => return div().into_any_element(),
        };
        div()
            .id("extension-composer-request")
            .role(Role::Group)
            .aria_label(title.clone())
            .track_focus(&self.dialog_focus)
            .key_context(super::OVERLAY_KEY_CONTEXT)
            .flex_none()
            .min_h(THEME.layout.composer_min)
            .max_h(THEME.layout.dialog_max_height)
            .overflow_y_scroll()
            .border_t(THEME.border)
            .border_color(THEME.colors.accent)
            .bg(THEME.colors.panel)
            .child(
                div()
                    .px(THEME.space.md)
                    .pt(THEME.space.sm)
                    .pb(THEME.space.xs)
                    .text_size(THEME.type_scale.body)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(div().px(THEME.space.md).pb(THEME.space.sm).child(body))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .px(THEME.space.md)
                    .pb(THEME.space.sm)
                    .child(button(
                        "dialog-cancel",
                        "Cancel",
                        ButtonTone::Quiet,
                        true,
                        move |window, cx| {
                            let _ = cancel_button_entity
                                .update(cx, |this, cx| this.cancel_dialog(window, cx));
                        },
                    )),
            )
            .on_mouse_down(MouseButton::Left, move |_, _, cx| cx.stop_propagation())
            .into_any_element()
    }
}

fn composer_status(app: &PiApp) -> Option<AnyElement> {
    let queued = app.snapshot.conversation.queue.steering.len()
        + app.snapshot.conversation.queue.follow_up.len();
    let mut parts = Vec::new();
    if !matches!(app.snapshot.status.as_str(), "" | "Ready" | "Idle" | "Done") {
        parts.push(app.snapshot.status.clone());
    }
    parts.extend(app.extension.statuses.values().cloned());
    if let Some(error) = app.extension_errors.last() {
        parts.push(error.chars().take(120).collect());
    }
    if queued > 0 {
        parts.push(format!("{queued} queued"));
    }
    if parts.is_empty() {
        return None;
    }
    Some(
        div()
            .px(THEME.space.sm)
            .pb(THEME.space.xs)
            .text_size(THEME.type_scale.caption)
            .text_color(
                if app.snapshot.status == "Failed" || !app.extension_errors.is_empty() {
                    THEME.colors.error
                } else {
                    THEME.colors.subtle
                },
            )
            .child(parts.join(" · "))
            .into_any_element(),
    )
}

fn capture_after_input(entity: WeakEntity<PiApp>, cx: &mut App) {
    cx.defer(move |cx| {
        let _ = entity.update(cx, |this, cx| this.capture_composer_session(cx));
    });
}

fn dialog_choice(
    id: impl Into<ElementId>,
    label: &str,
    primary: bool,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let (title, detail) = choice_copy(label);
    let on_press = Rc::new(on_press);
    let keyboard_press = on_press.clone();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.to_owned())
        .tab_index(0)
        .w_full()
        .min_h(px(48.0))
        .flex()
        .items_center()
        .gap(THEME.space.sm)
        .px(THEME.space.sm)
        .py(THEME.space.xs)
        .rounded(THEME.radius)
        .border(THEME.border)
        .border_color(if primary {
            THEME.colors.accent
        } else {
            THEME.colors.border
        })
        .bg(if primary {
            THEME.colors.selection
        } else {
            THEME.colors.surface
        })
        .hover(|choice| choice.bg(THEME.colors.hover))
        .focus(|choice| choice.border_color(THEME.colors.accent))
        .cursor(CursorStyle::PointingHand)
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                window.prevent_default();
                keyboard_press(window, cx);
            }
        })
        .on_click(move |_, window, cx| on_press(window, cx))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .gap(THEME.space.xs)
                .child(
                    div()
                        .font_weight(if primary {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::MEDIUM
                        })
                        .text_color(THEME.colors.text)
                        .child(title),
                )
                .when_some(detail, |copy, detail| {
                    copy.child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(detail),
                    )
                }),
        )
}

pub(super) fn dialog_copy(value: &str) -> (SharedString, Option<SharedString>) {
    let mut lines = value.lines();
    let heading = match lines.next().unwrap_or_default().trim() {
        "Tool requests an IO right" | "Tool requests grouped IO rights" => "File access request",
        "" => "Action required",
        heading => heading,
    };
    let prompt = lines.collect::<Vec<_>>().join("\n");
    let prompt = prompt
        .trim()
        .replace(" to access write file ", " to write to ")
        .replace(" to access read file ", " to read ");
    (
        heading.to_owned().into(),
        (!prompt.is_empty()).then(|| prompt.into()),
    )
}

pub(super) fn choice_copy(value: &str) -> (SharedString, Option<SharedString>) {
    let (title, detail) = match value {
        "Allow once and retry" => ("Allow once", Some("Retry this command")),
        "Always allow in this workspace and retry" => (
            "Always allow",
            Some("Remember for this workspace and retry"),
        ),
        "Allow once" => ("Allow once", None),
        "Always allow in this workspace" => ("Always allow", Some("Remember for this workspace")),
        "No" => ("Deny", None),
        "No, with comment" => ("Deny with note", Some("Tell Pi what to do instead")),
        value => (value, None),
    };
    (title.to_owned().into(), detail.map(Into::into))
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
