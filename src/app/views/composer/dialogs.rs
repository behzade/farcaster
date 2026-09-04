use gpui::{
    AnyElement, App, CursorStyle, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{input::Textarea, text::TextView};

use super::super::{FarcasterApp, OVERLAY_KEY_CONTEXT};
use crate::{
    app::ui::primitives::{ButtonTone, button},
    app::ui::theme::{MONO_FONT_FAMILY, THEME},
    app::views::transcript::conversation,
    protocol::ExtensionUiRequest,
};

impl FarcasterApp {
    pub(in crate::app::views) fn render_composer_request(
        &self,
        entity: WeakEntity<Self>,
    ) -> AnyElement {
        let Some(dialog) = self.extension.dialog.as_ref() else {
            return div().into_any_element();
        };
        let Some(id) = dialog.dialog_id().map(str::to_owned) else {
            return div().into_any_element();
        };
        let cancel_button_entity = entity.clone();
        let key_entity = entity.clone();
        let key_focus = self.dialog_focus.clone();
        let keyboard_dialog = dialog.clone();
        let technical_editor = matches!(dialog, ExtensionUiRequest::Editor { .. });
        let (title, body) = match dialog {
            ExtensionUiRequest::Select { title, options, .. } => {
                let choices = options
                    .iter()
                    .enumerate()
                    .map(|(index, option)| {
                        let value = option.clone();
                        let id = id.clone();
                        let choice_entity = entity.clone();
                        let label = numbered_dialog_choice(index, option);
                        dialog_choice(
                            ("dialog-option", index),
                            label.into(),
                            index == 0,
                            move |window, cx| {
                                let _ = choice_entity.update(cx, |this, cx| {
                                    this.respond_dialog_value(
                                        id.clone(),
                                        value.clone(),
                                        window,
                                        cx,
                                    );
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
                                selectable_dialog_text("dialog-select-prompt", prompt)
                                    .text_size(THEME.type_scale.body)
                                    .text_color(THEME.colors.muted)
                                    .line_height(THEME.type_scale.line_composer),
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
                let (reason, command) = conversation::split_command_block(message)
                    .map_or((message.as_str(), None), |(reason, command)| {
                        (reason, Some(command))
                    });
                (
                    SharedString::from(title.clone()),
                    div()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.md)
                        .when(!reason.is_empty(), |body| {
                            body.child(
                                selectable_dialog_text("dialog-confirm-message", reason)
                                    .text_size(THEME.type_scale.body)
                                    .text_color(THEME.colors.muted),
                            )
                        })
                        .when_some(command, |body, command| {
                            body.child(
                                selectable_dialog_text("dialog-confirm-command", command)
                                    .font_family(MONO_FONT_FAMILY)
                                    .text_size(THEME.type_scale.body_small)
                                    .text_color(THEME.colors.text),
                            )
                        })
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
                                selectable_dialog_text("dialog-input-hint", hint)
                                    .text_size(THEME.type_scale.caption)
                                    .text_color(THEME.colors.subtle),
                            )
                        })
                        .child(
                            div()
                                .when(technical_editor, |input| {
                                    input.font_family(MONO_FONT_FAMILY)
                                })
                                .child(
                                    Textarea::new(&self.dialog_input)
                                        .w_full()
                                        .into_any_element(),
                                ),
                        )
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
            .key_context(OVERLAY_KEY_CONTEXT)
            .capture_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified() || !key_focus.is_focused(window) {
                    return;
                }
                let selection = if event.keystroke.key == "enter" {
                    default_dialog_selection(&keyboard_dialog)
                } else {
                    dialog_number_selection(&keyboard_dialog, &event.keystroke.key)
                };
                if let Some((id, value)) = selection {
                    let id = id.to_owned();
                    let value = value.to_owned();
                    let _ = key_entity.update(cx, |this, cx| {
                        this.respond_dialog_value(id, value, window, cx);
                    });
                    window.prevent_default();
                    cx.stop_propagation();
                }
            })
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
                    .child(
                        selectable_dialog_text("extension-composer-request-title", title)
                            .text_size(THEME.type_scale.body)
                            .font_weight(FontWeight::SEMIBOLD),
                    ),
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
            .into_any_element()
    }
}

fn selectable_dialog_text(id: impl Into<ElementId>, text: impl AsRef<str>) -> TextView {
    TextView::html(id, plain_text_html(text.as_ref()))
        .selectable(true)
        .w_full()
        .min_w_0()
        .line_height(THEME.type_scale.line_body)
}

pub(super) fn plain_text_html(text: &str) -> SharedString {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            '\n' => escaped.push_str("<br>"),
            _ => escaped.push(character),
        }
    }
    escaped.into()
}

fn dialog_choice(
    id: impl Into<ElementId>,
    label: SharedString,
    primary: bool,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let (title, detail) = choice_copy(&label);
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
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
    let heading = lines.next().unwrap_or_default().trim();
    let prompt = lines.collect::<Vec<_>>().join("\n");
    (
        if heading.is_empty() {
            "Action required".into()
        } else {
            heading.to_owned().into()
        },
        (!prompt.trim().is_empty()).then(|| prompt.trim().to_owned().into()),
    )
}

pub(super) fn choice_copy(value: &str) -> (SharedString, Option<SharedString>) {
    (value.to_owned().into(), None)
}

pub(super) fn numbered_dialog_choice(index: usize, value: &str) -> String {
    format!("{}. {value}", index + 1)
}

pub(super) fn default_dialog_selection(request: &ExtensionUiRequest) -> Option<(&str, &str)> {
    match request {
        ExtensionUiRequest::Select { id, options, .. } => {
            options.first().map(|option| (id.as_str(), option.as_str()))
        }
        _ => None,
    }
}

pub(super) fn dialog_number_selection<'a>(
    request: &'a ExtensionUiRequest,
    key: &str,
) -> Option<(&'a str, &'a str)> {
    let index = match key {
        "1" => 0,
        "2" => 1,
        "3" => 2,
        "4" => 3,
        "5" => 4,
        _ => return None,
    };
    match request {
        ExtensionUiRequest::Select { id, options, .. } => options
            .get(index)
            .map(|option| (id.as_str(), option.as_str())),
        _ => None,
    }
}
