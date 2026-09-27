use gpui::{
    AnyElement, App, ElementId, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, prelude::FluentBuilder as _,
};
use gpui_component::input::{Input, InputState};

use crate::app::ui::{assets::AppIcon, theme::theme};

use super::icon::{AppIconSize, app_icon};

#[derive(IntoElement)]
pub(crate) struct SearchField {
    id: ElementId,
    input: Entity<InputState>,
    accessible_label: Option<SharedString>,
    trailing: Option<AnyElement>,
}

impl SearchField {
    pub(crate) fn new(id: impl Into<ElementId>, input: &Entity<InputState>) -> Self {
        Self {
            id: id.into(),
            input: input.clone(),
            accessible_label: None,
            trailing: None,
        }
    }

    pub(crate) fn accessible_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessible_label = Some(label.into());
        self
    }

    pub(crate) fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing = Some(element.into_any_element());
        self
    }
}

impl RenderOnce for SearchField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            input,
            accessible_label,
            trailing,
        } = self;
        let accessible = accessible_label.as_ref().map(SharedString::to_string);
        let focus_input = input.clone();
        let focused = input.read(cx).focus_handle(cx).is_focused(window);
        div()
            .id(id)
            .h(theme().size(36.0))
            .flex_none()
            .flex()
            .gap(theme().space.xs)
            .pl(theme().size(10.0))
            .border_b(theme().border)
            .border_color(if focused {
                theme().colors.border
            } else {
                gpui::rgba(0x00000000)
            })
            .text_color(theme().colors.muted)
            .when_some(accessible, |field, label| field.aria_label(label))
            .on_click(move |_, window, cx| {
                focus_input.read(cx).focus_handle(cx).focus(window, cx);
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(theme().space.xs)
                    .child(app_icon(AppIcon::MagnifyingGlass, AppIconSize::Inline))
                    .child(Input::new(&input).flex_1().min_w_0().appearance(false)),
            )
            .children(trailing)
    }
}
