mod confirm;
mod select;
mod text_input;

use gpui::{
    AnyElement, ElementId, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, div, prelude::FluentBuilder as _,
};
use gpui_component::popover::Popover;
use gpui_component::text::TextView;

use self::{confirm::ConfirmRequestView, select::SelectRequestView, text_input::TextRequestView};
use super::super::{FarcasterApp, OVERLAY_KEY_CONTEXT};
use crate::{
    app::ui::primitives::{ButtonTone, button, dropdown_content_button},
    app::ui::theme::theme,
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
        focused: bool,
    ) -> AnyElement {
        let Some(dialog) = self.extensions.active.dialog.as_ref() else {
            return div().into_any_element();
        };
        if dialog.dialog_id().is_none() {
            return div().into_any_element();
        }
        let (title, body) = match dialog {
            ExtensionUiRequest::WorkerModel { id, profile, .. } => {
                let title = format!("{profile} needs a model");
                let picker_entity = entity.clone();
                let content_entity = entity.clone();
                let picker_id = id.clone();
                let picker = Popover::new("worker-model-popover")
                    .anchor(gpui::Anchor::BottomLeft)
                    .appearance(false)
                    .open(
                        self.workspace.runtime_picker.open
                            && self.workspace.runtime_picker.worker.is_some(),
                    )
                    .trigger(dropdown_content_button(
                        "choose-worker-model",
                        "Choose model",
                        div().child("Choose model"),
                        ButtonTone::Neutral,
                        true,
                    ))
                    .on_open_change(move |open, window, cx| {
                        let _ = picker_entity.update(cx, |app, cx| {
                            app.set_worker_model_picker_open(*open, &picker_id, window, cx)
                        });
                    })
                    .content(move |_, window, cx| {
                        content_entity
                            .update(cx, |app, cx| app.render_runtime_picker(window, cx))
                            .unwrap_or_else(|_| div().into_any_element())
                    });
                let body = div()
                    .flex()
                    .flex_col()
                    .gap(theme().space.sm)
                    .child(
                        div()
                            .text_color(theme().colors.muted)
                            .child("Choose a model to start this worker."),
                    )
                    .child(picker);
                (title.into(), body.into_any_element())
            }
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
                    self.extensions.dialog_input.clone(),
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
                    self.extensions.dialog_input.clone(),
                    entity.clone(),
                );
                (view.title().clone(), view.into_any_element())
            }
            _ => return div().into_any_element(),
        };

        let cancel_button_entity = entity.clone();
        let key_entity = entity;
        let key_focus = self.extensions.dialog_focus.clone();
        let keyboard_dialog = dialog.clone();

        div()
            .id("extension-composer-request")
            .role(Role::Group)
            .aria_label(title.clone())
            .track_focus(&self.extensions.dialog_focus)
            .key_context(OVERLAY_KEY_CONTEXT)
            .capture_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified() || !key_focus.contains_focused(window, cx) {
                    return;
                }
                if matches!(
                    keyboard_dialog,
                    ExtensionUiRequest::Select { .. } | ExtensionUiRequest::Confirm { .. }
                ) && (event.is_held
                    || matches!(event.keystroke.key.as_str(), "enter" | "space" | " "))
                {
                    window.prevent_default();
                    cx.stop_propagation();
                    return;
                }
                if let Some((id, confirmed)) =
                    dialog_confirmation(&keyboard_dialog, &event.keystroke.key)
                {
                    let id = id.to_owned();
                    let _ = key_entity.update(cx, |this, cx| {
                        this.respond_confirm(id, confirmed, window, cx);
                    });
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
            .min_h(theme().layout.composer_min)
            .max_h(theme().layout.dialog_max_height)
            .overflow_y_scroll()
            .border_t(theme().border)
            .border_color(super::composer_border_color(focused))
            .bg(theme().colors.panel)
            .child(
                div()
                    .px(theme().space.md)
                    .pt(theme().space.sm)
                    .pb(theme().space.xs)
                    .child(
                        selectable_dialog_text("extension-composer-request-title", title)
                            .text_size(theme().type_scale.body)
                            .font_weight(FontWeight::SEMIBOLD),
                    ),
            )
            .child(div().px(theme().space.md).pb(theme().space.sm).child(body))
            .when(show_cancel_button(dialog), |body| {
                body.child(
                    div()
                        .flex()
                        .justify_end()
                        .px(theme().space.md)
                        .pb(theme().space.sm)
                        .child(button(
                            "dialog-cancel",
                            if matches!(dialog, ExtensionUiRequest::WorkerModel { .. }) {
                                "Cancel request"
                            } else {
                                "Cancel"
                            },
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

fn dialog_confirmation<'a>(dialog: &'a ExtensionUiRequest, key: &str) -> Option<(&'a str, bool)> {
    let ExtensionUiRequest::Confirm { id, .. } = dialog else {
        return None;
    };
    match key {
        "n" => Some((id, false)),
        "y" => Some((id, true)),
        _ => None,
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
        .line_height(theme().type_scale.line_body)
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
#[path = "dialogs_tests.rs"]
mod cancel_button_tests;
