use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    RenderOnce, Role, SharedString, StatefulInteractiveElement as _, Styled as _, StyledText,
    Window, div, prelude::FluentBuilder as _,
};

use crate::app::{
    composer::queue::{QueueAction, QueueActionHandler, QueuePresentation, QueueRow},
    ui::{
        assets::AppIcon,
        primitives::{
            AppIconSize, AppTooltip as _, ButtonTone, activates_button, app_icon, button,
            icon_control, preserve_pointer_focus,
        },
        theme::theme,
    },
};

pub(super) fn message_body(id: String, text: &str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(id))
        .debug_selector(|| "queue-message-body".into())
        .w_full()
        .min_w_0()
        .max_h(theme().layout.tool_max_height)
        .overflow_scroll()
        .text_color(theme().colors.text)
        .child(text.to_owned())
}

#[derive(Default)]
struct Expansion {
    preview: String,
    body: String,
    clipped: bool,
    expanded: bool,
}

#[derive(IntoElement)]
struct QueueMessageRow {
    row: QueueRow,
    on_action: QueueActionHandler,
}

impl RenderOnce for QueueMessageRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let row = self.row;
        let state = window.use_keyed_state(SharedString::from(row.id.clone()), cx, |_, _| {
            Expansion::default()
        });
        state.update(cx, |state, _| {
            if state.preview != row.preview || state.body != row.body {
                *state = Expansion {
                    preview: row.preview.clone(),
                    body: row.body.clone(),
                    ..Default::default()
                };
            }
        });
        let has_details = row.preview != row.body || row.details.is_some();
        let expandable = has_details || state.read(cx).clipped;
        let expanded = expandable && state.read(cx).expanded;
        let click = state.clone();
        let keyboard = state.clone();
        let preview = StyledText::new(row.preview.clone());
        let layout = preview.layout().clone();
        let preview_text = row.preview.clone();
        let label = format!("Inspect message: {}", row.preview);
        let notice_id = SharedString::from(format!("{}-notice", row.id));
        let selector = row.id.clone();
        div()
            .id(SharedString::from(row.id.clone()))
            .debug_selector(move || selector.clone())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .text_size(theme().type_scale.caption)
            .text_color(theme().colors.muted)
            .child(
                div()
                    .min_h(theme().layout.status_row_height)
                    .flex()
                    .items_center()
                    .gap(theme().space.xs)
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-preview", row.id)))
                            .debug_selector({
                                let id = row.id.clone();
                                move || format!("{id}-preview")
                            })
                            .flex_1()
                            .min_w_0()
                            .min_h(theme().layout.status_row_height)
                            .flex()
                            .items_center()
                            .when(expandable, |preview| {
                                preview
                                    .role(Role::Button)
                                    .aria_label(label)
                                    .aria_expanded(expanded)
                                    .tab_index(0)
                                    .cursor_pointer()
                                    .hover(|row| row.text_color(theme().colors.text))
                                    .focus_visible(|row| row.bg(theme().colors.highlight))
                                    .on_mouse_down(MouseButton::Left, preserve_pointer_focus)
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        click.update(cx, |state, cx| {
                                            state.expanded = !state.expanded;
                                            cx.notify();
                                        });
                                    })
                                    .on_key_down(move |event, _, cx| {
                                        if activates_button(event) {
                                            cx.stop_propagation();
                                            keyboard.update(cx, |state, cx| {
                                                state.expanded = !state.expanded;
                                                cx.notify();
                                            });
                                        }
                                    })
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .on_children_prepainted(move |_, _, cx| {
                                        // Use the actual truncated layout, including the current font and width.
                                        let clipped = layout.text() != preview_text;
                                        state.update(cx, |state, cx| {
                                            if state.clipped != clipped {
                                                state.clipped = clipped;
                                                if !has_details && !clipped {
                                                    state.expanded = false;
                                                }
                                                cx.notify();
                                            }
                                        });
                                    })
                                    .child(preview),
                            ),
                    )
                    .when_some(row.notice, |row, notice| {
                        row.child(
                            div()
                                .id(notice_id)
                                .debug_selector(|| "queue-row-notice".into())
                                .flex_none()
                                .text_color(theme().colors.warning)
                                .app_tooltip(notice)
                                .child(app_icon(AppIcon::WarningCircle, AppIconSize::Inline)),
                        )
                    })
                    .children(
                        row.action
                            .map(|action| action_icon(action, AppIcon::X, self.on_action.clone())),
                    ),
            )
            .when(expanded, |element| {
                let details = row.details;
                let notice = details.as_ref().map(|details| details.notice);
                let caption = details.as_ref().and_then(|details| details.caption.clone());
                element.child(
                    div()
                        .when_some(notice, |body, notice| {
                            body.child(div().text_color(theme().colors.warning).child(notice))
                        })
                        .child(message_body(format!("{}-body", row.id), &row.body))
                        .children(caption)
                        .when_some(details, |body, details| {
                            body.child(div().flex().gap(theme().space.xs).children(
                                details.actions.into_iter().map(|action| {
                                    let on_action = self.on_action.clone();
                                    let selector = action.id.clone();
                                    button(
                                        SharedString::from(action.id.clone()),
                                        action.label,
                                        ButtonTone::Quiet,
                                        action.enabled,
                                        move |_, cx| on_action(&action, cx),
                                    )
                                    .debug_selector(move || selector.clone())
                                }),
                            ))
                        }),
                )
            })
    }
}

fn action_icon(action: QueueAction, icon: AppIcon, on_action: QueueActionHandler) -> AnyElement {
    let click = on_action.clone();
    let click_action = action.clone();
    let selector = action.id.clone();
    icon_control(SharedString::from(action.id.clone()), action.label)
        .h(theme().layout.status_row_height)
        .debug_selector(move || selector.clone())
        .child(app_icon(icon, AppIconSize::Inline))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            click(&click_action, cx);
        })
        .on_key_down(move |event, _, cx| {
            if activates_button(event) {
                cx.stop_propagation();
                on_action(&action, cx);
            }
        })
        .into_any_element()
}

pub(super) fn render(
    presentation: QueuePresentation,
    on_action: QueueActionHandler,
) -> Option<AnyElement> {
    if presentation.groups.is_empty() {
        return None;
    }
    let mut clear = presentation
        .clear
        .map(|action| action_icon(action, AppIcon::Trash, on_action.clone()));
    Some(
        div()
            .id("composer-queue-tray")
            .debug_selector(|| "composer-queue-tray".into())
            .w_full()
            .min_w_0()
            .mb(theme().space.xs)
            .px(theme().space.sm)
            .max_h(theme().layout.tool_max_height)
            .overflow_y_scroll()
            .children(presentation.groups.into_iter().map(|group| {
                div()
                    .when_some(group.heading, |element, (heading, tooltip)| {
                        element.child(
                            div()
                                .id(SharedString::from(group.id.clone()))
                                .debug_selector(move || group.id.clone())
                                .flex()
                                .items_center()
                                .justify_between()
                                .py(theme().space.xs)
                                .text_size(theme().type_scale.caption)
                                .text_color(theme().colors.subtle)
                                .app_tooltip(tooltip)
                                .child(heading)
                                .children(clear.take()),
                        )
                    })
                    .children(group.rows.into_iter().map(|row| QueueMessageRow {
                        row,
                        on_action: on_action.clone(),
                    }))
            }))
            .into_any_element(),
    )
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
