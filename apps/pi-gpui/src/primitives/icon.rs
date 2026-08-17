use gpui::Pixels;
use gpui::{
    CursorStyle, Div, ElementId, InteractiveElement as _, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, div,
};
use gpui_component::{Icon, IconNamed, Sizable as _, tooltip::Tooltip};

use crate::theme::THEME;

#[derive(Clone, Copy)]
pub(crate) enum AppIconSize {
    Inline,
    Control,
    Prominent,
}

impl AppIconSize {
    fn pixels(self) -> Pixels {
        match self {
            Self::Inline => THEME.icons.inline,
            Self::Control => THEME.icons.control,
            Self::Prominent => THEME.icons.prominent,
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
        .size(THEME.controls.icon_button)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(THEME.radius)
        .focus_visible(|control| {
            control
                .border(THEME.border)
                .border_color(THEME.colors.accent)
        })
        .cursor(CursorStyle::PointingHand)
        .tooltip(move |window, cx| Tooltip::new(tooltip_label.clone()).build(window, cx))
}
