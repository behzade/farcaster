use gpui::{
    AnyElement, App, CursorStyle, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, point,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    input::{Input, MoveDown, MoveUp, Paste, Textarea},
    text::TextView,
};

use super::super::{
    ComposerHistoryNext, ComposerHistoryPrevious, PiApp, file_mentions, slash_commands,
};
use crate::{
    app::file_mentions::MentionQuery,
    composer_sessions::ComposerSnapshot,
    conversation::QueueState,
    primitives::{ButtonTone, button},
    protocol::ExtensionUiRequest,
    theme::{MONO_FONT_FAMILY, THEME},
    user_invocations::{self, ComposerSuggestion},
};

impl PiApp {
    pub(super) fn render_composer(&self, entity: WeakEntity<Self>, cx: &App) -> AnyElement {
        if self.extension.dialog.is_some() {
            return self.render_composer_request(entity);
        }
        if self.extension.provider_auth.is_some() {
            return self.render_provider_auth();
        }
        let floating = self.selected_draft_is_empty_and_unsubmitted();
        let composer = self.composer.read(cx);
        let composer_text = composer.value().to_string();
        let composer_cursor = composer.cursor().min(composer_text.len());
        let composer_value = composer_text.trim().to_owned();
        let command_suggestions =
            slash_commands::suggestions(composer_text.trim_start(), &self.snapshot.commands)
                .into_iter()
                .chain(user_invocations::suggestions(
                    &composer_text[..composer_cursor],
                    &self.snapshot.commands,
                ))
                .take(8)
                .collect::<Vec<_>>();
        let command_suggestion_count = command_suggestions.len();
        let exact_command = slash_commands::is_exact(&composer_value, &self.snapshot.commands);
        let primary_action = composer_primary_action(
            !composer_value.is_empty() || self.has_composer_images(),
            self.can_submit(),
            exact_command,
            self.snapshot.conversation.running,
        );
        let mention_query = file_mentions::query_at_cursor(
            &self.composer.read(cx).value(),
            self.composer.read(cx).cursor(),
        );
        let file_suggestions = mention_query
            .as_ref()
            .map(|query| file_mentions::matches(&self.composer_project_files, &query.text))
            .unwrap_or_default();
        let widgets_above = widget_region("above", &self.extension.above_widgets);
        let widgets_below = widget_region("below", &self.extension.below_widgets);
        let previous_history_entity = entity.clone();
        let next_history_entity = entity.clone();
        let paste_entity = entity.clone();
        let composer_for_paste = self.composer.clone();
        let key_entity = entity.clone();
        let cursor_entity = entity.clone();
        let attachments_entity = entity.clone();
        let command_entity = entity.clone();
        let mention_entity = entity.clone();
        let suggestion_selection = self.composer_suggestion_selection;
        let mention_selection = suggestion_selection.min(file_suggestions.len().saturating_sub(1));
        let command_selection =
            suggestion_selection.min(command_suggestion_count.saturating_sub(1));
        let mention_suggestion_count = file_suggestions.len();
        let controls_entity = entity.clone();
        let actions_entity = entity;
        div()
            .w_full()
            .flex_none()
            .min_h(THEME.layout.composer_min)
            .flex()
            .flex_col()
            .overflow_hidden()
            .when(floating, |composer| {
                composer
                    .rounded(THEME.radius)
                    .border(THEME.border)
                    .border_color(THEME.colors.border)
            })
            .when(!floating, |composer| {
                composer
                    .border_t(THEME.border)
                    .border_color(THEME.colors.border)
            })
            .bg(THEME.colors.panel)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .p(THEME.space.sm)
                    .when_some(widgets_above, |composer, widgets| composer.child(widgets))
                    .when_some(
                        queued_messages(&self.snapshot.conversation.queue),
                        |composer, queue| composer.child(queue),
                    )
                    .when_some(
                        super::attachments::render(self, attachments_entity),
                        |composer, attachments| composer.child(attachments),
                    )
                    .when(!command_suggestions.is_empty(), |composer| {
                        composer.child(command_menu(
                            command_suggestions,
                            command_selection,
                            command_entity,
                        ))
                    })
                    .when_some(
                        mention_query
                            .clone()
                            .filter(|_| !file_suggestions.is_empty()),
                        |composer, query| {
                            composer.child(file_mention_menu(
                                file_suggestions,
                                mention_selection,
                                query,
                                mention_entity,
                            ))
                        },
                    )
                    .child(
                        div()
                            .id("composer-input")
                            .key_context(super::super::COMPOSER_KEY_CONTEXT)
                            .flex_1()
                            .min_h(px(112.0))
                            .font_family(MONO_FONT_FAMILY)
                            .text_size(THEME.type_scale.body)
                            .line_height(THEME.type_scale.line_composer)
                            .px(THEME.space.sm)
                            .py(THEME.space.sm)
                            .capture_action(move |_: &Paste, _, cx| {
                                if paste_entity
                                    .update(cx, |this, cx| this.paste_composer_image(cx))
                                    .unwrap_or(false)
                                {
                                    cx.stop_propagation();
                                    return;
                                }

                                let composer = composer_for_paste.clone();
                                cx.defer(move |cx| {
                                    composer.update(cx, |input, cx| {
                                        let offset = input.scroll_offset();
                                        input.set_scroll_offset(point(offset.x, px(-1.0e9)), cx);
                                    });
                                });
                            })
                            .on_action(move |_: &ComposerHistoryPrevious, window, cx| {
                                let handled = previous_history_entity
                                    .update(cx, |this, cx| {
                                        let suggestion_count = if mention_suggestion_count > 0 {
                                            mention_suggestion_count
                                        } else {
                                            command_suggestion_count
                                        };
                                        if suggestion_count > 0 {
                                            this.composer_suggestion_selection = this
                                                .composer_suggestion_selection
                                                .checked_sub(1)
                                                .unwrap_or(suggestion_count - 1);
                                            this.notify_composer(cx);
                                            true
                                        } else {
                                            this.handle_composer_history_key("up", window, cx)
                                        }
                                    })
                                    .unwrap_or(false);
                                if !handled {
                                    window.dispatch_action(Box::new(MoveUp), cx);
                                }
                                cx.stop_propagation();
                            })
                            .on_action(move |_: &ComposerHistoryNext, window, cx| {
                                let handled = next_history_entity
                                    .update(cx, |this, cx| {
                                        let suggestion_count = if mention_suggestion_count > 0 {
                                            mention_suggestion_count
                                        } else {
                                            command_suggestion_count
                                        };
                                        if suggestion_count > 0 {
                                            this.composer_suggestion_selection =
                                                (this.composer_suggestion_selection + 1)
                                                    % suggestion_count;
                                            this.notify_composer(cx);
                                            true
                                        } else {
                                            this.handle_composer_history_key("down", window, cx)
                                        }
                                    })
                                    .unwrap_or(false);
                                if !handled {
                                    window.dispatch_action(Box::new(MoveDown), cx);
                                }
                                cx.stop_propagation();
                            })
                            .capture_key_down(move |_: &KeyDownEvent, _, cx| {
                                capture_after_input(key_entity.clone(), cx);
                            })
                            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                                capture_after_input(cursor_entity.clone(), cx);
                            })
                            .child(Textarea::new(&self.composer).w_full().appearance(false)),
                    )
                    .when_some(widgets_below, |composer, widgets| composer.child(widgets)),
            )
            .child(
                div()
                    .min_h(THEME.controls.utility_row)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(THEME.space.md)
                    .py(THEME.space.sm)
                    .border_t(THEME.border)
                    .border_color(THEME.colors.border)
                    .child(self.render_composer_controls(controls_entity, !floating))
                    .child(self.render_composer_actions(actions_entity, primary_action)),
            )
            .into_any_element()
    }

    fn render_provider_auth(&self) -> AnyElement {
        let Some(auth) = self.extension.provider_auth.as_ref() else {
            return div().into_any_element();
        };
        let url = auth.url.clone();
        div()
            .id("provider-auth-request")
            .role(Role::Group)
            .aria_label("Complete provider sign-in")
            .flex_none()
            .min_h(THEME.layout.composer_min)
            .border_t(THEME.border)
            .border_color(THEME.colors.accent)
            .bg(THEME.colors.panel)
            .p(THEME.space.md)
            .flex()
            .flex_col()
            .gap(THEME.space.md)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Complete provider sign-in"),
            )
            .child(selectable_dialog_text(
                "provider-auth-instructions",
                auth.message.clone(),
            ))
            .child(
                div()
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.subtle)
                    .child(
                        "Waiting for browser authorization. This panel closes automatically when sign-in finishes.",
                    ),
            )
            .child(div().flex().justify_end().child(button(
                "provider-auth-open-browser",
                "Open browser",
                ButtonTone::Accent,
                true,
                move |_, cx| cx.open_url(&url),
            )))
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
        let key_entity = entity.clone();
        let key_focus = self.dialog_focus.clone();
        let keyboard_dialog = dialog.clone();
        let technical_editor = matches!(dialog, ExtensionUiRequest::Editor { .. });
        let secret_input = matches!(dialog, ExtensionUiRequest::Secret { .. });
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
                            &label,
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
                (
                    SharedString::from(title.clone()),
                    div()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.md)
                        .child(selectable_dialog_text(
                            "dialog-confirm-message",
                            message.clone(),
                        ))
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
            | ExtensionUiRequest::Secret {
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
                                .child(if secret_input {
                                    Input::new(&self.dialog_secret_input)
                                        .w_full()
                                        .into_any_element()
                                } else {
                                    Textarea::new(&self.dialog_input)
                                        .w_full()
                                        .into_any_element()
                                }),
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
            .key_context(super::OVERLAY_KEY_CONTEXT)
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

fn file_mention_menu(
    files: Vec<String>,
    selected: usize,
    query: MentionQuery,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let mut menu = div()
        .id("file-mention-menu")
        .role(Role::Group)
        .aria_label("Repository files")
        .max_h(px(220.0))
        .overflow_y_scroll()
        .mb(THEME.space.sm)
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .rounded(THEME.radius)
        .bg(THEME.colors.surface)
        .p(THEME.space.xs);
    for (index, path) in files.into_iter().enumerate() {
        let click_entity = entity.clone();
        let click_query = query.clone();
        menu = menu.child(
            div()
                .id(("file-mention", index))
                .role(Role::Button)
                .aria_label(format!("Mention {path}"))
                .tab_index(0)
                .px(THEME.space.sm)
                .py(THEME.space.xs)
                .rounded(THEME.radius)
                .font_family(MONO_FONT_FAMILY)
                .text_size(THEME.type_scale.caption)
                .when(index == selected, |row| {
                    row.bg(THEME.colors.hover).text_color(THEME.colors.accent)
                })
                .hover(|row| row.bg(THEME.colors.hover))
                .focus(|row| row.border(THEME.border).border_color(THEME.colors.accent))
                .cursor_pointer()
                .child(path.clone())
                .on_click(move |_, window, cx| {
                    fill_file_mention(
                        click_entity.clone(),
                        click_query.clone(),
                        path.clone(),
                        window,
                        cx,
                    );
                }),
        );
    }
    menu.into_any_element()
}

fn fill_file_mention(
    entity: WeakEntity<PiApp>,
    query: MentionQuery,
    path: String,
    window: &mut Window,
    cx: &mut App,
) {
    let _ = entity.update(cx, |this, cx| {
        let (text, cursor) = file_mentions::insert(&this.composer.read(cx).value(), &query, &path);
        this.apply_composer_snapshot(
            ComposerSnapshot::new(text, cursor, cursor..cursor),
            window,
            cx,
        );
        this.composer_focus.focus(window, cx);
    });
}

fn command_menu(
    commands: Vec<ComposerSuggestion>,
    selected: usize,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let mut menu = div()
        .id("command-menu")
        .role(Role::Group)
        .aria_label("Commands and user invocations")
        .max_h(px(220.0))
        .overflow_y_scroll()
        .mb(THEME.space.sm)
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .rounded(THEME.radius)
        .bg(THEME.colors.surface)
        .p(THEME.space.xs);
    for (index, command) in commands.into_iter().enumerate() {
        let name = command.name;
        let sigil = command.sigil;
        let click_entity = entity.clone();
        let click_name = name.clone();
        menu = menu.child(
            div()
                .id(("composer-command", index))
                .role(Role::Button)
                .aria_label(format!("Use {sigil}{name}"))
                .tab_index(0)
                .flex()
                .items_center()
                .gap(THEME.space.sm)
                .px(THEME.space.sm)
                .py(THEME.space.xs)
                .rounded(THEME.radius)
                .when(index == selected, |row| {
                    row.bg(THEME.colors.hover).text_color(THEME.colors.accent)
                })
                .hover(|row| row.bg(THEME.colors.hover))
                .focus(|row| row.border(THEME.border).border_color(THEME.colors.accent))
                .cursor_pointer()
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(THEME.colors.accent)
                        .child(format!("{sigil}{name}")),
                )
                .when_some(command.description, |row, description| {
                    row.child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.muted)
                            .child(description),
                    )
                })
                .on_click(move |_, window, cx| {
                    fill_command(click_entity.clone(), sigil, click_name.clone(), window, cx);
                }),
        );
    }
    menu.into_any_element()
}

fn fill_command(
    entity: WeakEntity<PiApp>,
    sigil: char,
    name: String,
    window: &mut Window,
    cx: &mut App,
) {
    let _ = entity.update(cx, |this, cx| {
        let composer = this.composer.read(cx);
        let (text, cursor) =
            user_invocations::complete(&composer.value(), composer.cursor(), sigil, &name);
        this.apply_composer_snapshot(
            ComposerSnapshot::new(text, cursor, cursor..cursor),
            window,
            cx,
        );
        this.composer_focus.focus(window, cx);
    });
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum QueuedMessageKind {
    Steer,
    FollowUp,
}

impl QueuedMessageKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Steer => "Steer next",
            Self::FollowUp => "Follow-ups",
        }
    }
}

pub(super) fn queued_message_groups(queue: &QueueState) -> Vec<(QueuedMessageKind, &[String])> {
    [
        (QueuedMessageKind::Steer, queue.steering.as_slice()),
        (QueuedMessageKind::FollowUp, queue.follow_up.as_slice()),
    ]
    .into_iter()
    .filter(|(_, messages)| !messages.is_empty())
    .collect()
}

fn queued_message_group(
    kind: QueuedMessageKind,
    messages: &[String],
    separated: bool,
) -> AnyElement {
    div()
        .when(separated, |group| {
            group
                .border_t(THEME.border)
                .border_color(THEME.colors.border)
        })
        .child(
            div()
                .px(THEME.space.sm)
                .py(THEME.space.xs)
                .bg(match kind {
                    QueuedMessageKind::Steer => THEME.colors.selection,
                    QueuedMessageKind::FollowUp => THEME.colors.hover,
                })
                .text_size(THEME.type_scale.caption)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(match kind {
                    QueuedMessageKind::Steer => THEME.colors.accent,
                    QueuedMessageKind::FollowUp => THEME.colors.subtle,
                })
                .child(kind.label()),
        )
        .children(messages.iter().map(|message| {
            div()
                .border_t(THEME.border)
                .border_color(THEME.colors.border)
                .px(THEME.space.sm)
                .py(THEME.space.xs)
                .text_size(THEME.type_scale.body)
                .text_color(THEME.colors.text)
                .child(message.clone())
        }))
        .into_any_element()
}

fn queued_messages(queue: &QueueState) -> Option<AnyElement> {
    let groups = queued_message_groups(queue);
    if groups.is_empty() {
        return None;
    }
    Some(
        div()
            .mb(THEME.space.sm)
            .border(THEME.border)
            .border_color(THEME.colors.border)
            .rounded(THEME.radius)
            .overflow_hidden()
            .bg(THEME.colors.surface)
            .children(
                groups
                    .into_iter()
                    .enumerate()
                    .map(|(index, (kind, messages))| {
                        queued_message_group(kind, messages, index > 0)
                    }),
            )
            .into_any_element(),
    )
}

pub(super) fn composer_primary_action(
    has_content: bool,
    can_submit: bool,
    exact_command: bool,
    running: bool,
) -> Option<&'static str> {
    if !has_content || !can_submit {
        return None;
    }
    Some(if exact_command {
        "Run"
    } else if running {
        "Steer"
    } else {
        "Send"
    })
}

fn capture_after_input(entity: WeakEntity<PiApp>, cx: &mut App) {
    cx.defer(move |cx| {
        let _ = entity.update(cx, |this, cx| this.capture_composer_session(cx));
    });
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
    label: &str,
    primary: bool,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let (title, detail) = choice_copy(label);
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
                    .font_family(MONO_FONT_FAMILY)
                    .text_size(THEME.type_scale.caption)
                    .children(lines.iter().cloned().map(|line| div().child(line)))
            }))
            .into_any_element(),
    )
}
