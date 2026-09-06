mod confirm;
mod select;
mod text_input;

use gpui::{
    AnyElement, ElementId, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, div, prelude::FluentBuilder as _,
};
use gpui_component::text::TextView;

use self::{confirm::ConfirmRequestView, select::SelectRequestView, text_input::TextRequestView};
use super::super::{FarcasterApp, OVERLAY_KEY_CONTEXT};
use crate::{
    app::ui::primitives::{ButtonTone, button},
    app::ui::theme::THEME,
    protocol::ExtensionUiRequest,
};

#[cfg(not(test))]
use select::dialog_number_selection;
#[cfg(test)]
pub(super) use select::{
    choice_copy, dialog_copy, dialog_number_selection, numbered_dialog_choice,
};

impl FarcasterApp {
    pub(in crate::app::views) fn render_composer_request(
        &self,
        entity: WeakEntity<Self>,
    ) -> AnyElement {
        let Some(dialog) = self.extension.dialog.as_ref() else {
            return div().into_any_element();
        };
        if dialog.dialog_id().is_none() {
            return div().into_any_element();
        }

        let (title, body) = match dialog {
            ExtensionUiRequest::Select {
                id, title, options, ..
            } => {
                let view = SelectRequestView::new(
                    id.clone(),
                    title.clone(),
                    options.clone(),
                    entity.clone(),
                );
                (view.title().clone(), view.into_any_element())
            }
            ExtensionUiRequest::Confirm {
                id, title, message, ..
            } => {
                let view = ConfirmRequestView::new(
                    id.clone(),
                    title.clone(),
                    message.clone(),
                    entity.clone(),
                );
                (view.title().clone(), view.into_any_element())
            }
            ExtensionUiRequest::Input {
                id,
                title,
                placeholder,
                ..
            } => {
                let view = TextRequestView::new(
                    id.clone(),
                    title.clone(),
                    placeholder.clone(),
                    false,
                    self.dialog_input.clone(),
                    entity.clone(),
                );
                (view.title().clone(), view.into_any_element())
            }
            ExtensionUiRequest::Editor { id, title, prefill } => {
                let view = TextRequestView::new(
                    id.clone(),
                    title.clone(),
                    prefill.clone(),
                    true,
                    self.dialog_input.clone(),
                    entity.clone(),
                );
                (view.title().clone(), view.into_any_element())
            }
            _ => return div().into_any_element(),
        };

        let cancel_button_entity = entity.clone();
        let key_entity = entity;
        let key_focus = self.dialog_focus.clone();
        let keyboard_dialog = dialog.clone();

        div()
            .id("extension-composer-request")
            .role(Role::Group)
            .aria_label(title.clone())
            .track_focus(&self.dialog_focus)
            .key_context(OVERLAY_KEY_CONTEXT)
            .capture_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified() || !key_focus.contains_focused(window, cx) {
                    return;
                }
                // Select choices require an explicit number, never implicit approval.
                if matches!(keyboard_dialog, ExtensionUiRequest::Select { .. })
                    && matches!(event.keystroke.key.as_str(), "enter" | "space" | " ")
                {
                    window.prevent_default();
                    cx.stop_propagation();
                    return;
                }
                let selection = dialog_number_selection(&keyboard_dialog, &event.keystroke.key);
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
            .when(show_cancel_button(dialog), |body| {
                body.child(
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
            })
            .into_any_element()
    }
}

fn show_cancel_button(dialog: &ExtensionUiRequest) -> bool {
    match dialog {
        ExtensionUiRequest::Confirm { .. } => false,
        ExtensionUiRequest::Select { options, .. } => !options.iter().any(|option| {
            ["deny", "reject", "cancel", "no"]
                .iter()
                .any(|label| option.trim().eq_ignore_ascii_case(label))
        }),
        _ => true,
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

#[cfg(test)]
mod cancel_button_tests {
    use super::*;

    #[test]
    fn cancel_is_shown_only_without_a_refusal_choice() {
        for (label, show_cancel) in [
            ("Deny", false),
            ("Reject", false),
            ("Cancel", false),
            ("No", false),
            (" deny ", false),
            ("Other", true),
        ] {
            let request = ExtensionUiRequest::Select {
                id: "request".into(),
                title: "Run command?".into(),
                options: vec!["Accept".into(), label.into()],
                timeout: None,
            };
            assert_eq!(show_cancel_button(&request), show_cancel, "{label}");
        }
    }

    #[test]
    fn confirmation_has_its_own_refusal() {
        let request = ExtensionUiRequest::Confirm {
            id: "request".into(),
            title: "Run command?".into(),
            message: String::new(),
            timeout: None,
        };
        assert!(!show_cancel_button(&request));
    }
}
