use gpui::{
    CursorStyle, Div, ElementId, InteractiveElement as _, MouseButton, Pixels, Role, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, div,
};
use gpui_component::{Icon, IconNamed, Sizable as _};

use super::AppTooltip as _;
use crate::app::ui::theme::theme;

#[derive(Clone, Copy)]
pub(crate) enum AppIconSize {
    Inline,
    Control,
    Prominent,
}

impl AppIconSize {
    fn pixels(self) -> Pixels {
        match self {
            Self::Inline => theme().icons.inline,
            Self::Control => theme().icons.control,
            Self::Prominent => theme().icons.prominent,
        }
    }
}

pub(crate) fn app_icon(icon: impl IconNamed, size: AppIconSize) -> Icon {
    Icon::new(icon).with_size(size.pixels())
}

pub(crate) fn icon_control(
    id: impl Into<ElementId>,
    accessible_label: impl Into<SharedString>,
) -> Stateful<Div> {
    let accessible_label = accessible_label.into();
    let tooltip_label = accessible_label.clone();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(accessible_label)
        .tab_index(0)
        .size(theme().controls.icon_button)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(theme().radius)
        .focus_visible(|control| {
            control
                .border(theme().border)
                .border_color(theme().colors.accent)
        })
        .cursor(CursorStyle::PointingHand)
        .on_mouse_down(MouseButton::Left, super::preserve_pointer_focus)
        .app_tooltip(tooltip_label)
}
