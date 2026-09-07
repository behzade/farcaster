use std::rc::Rc;

use gpui::{
    AnyElement, App, CursorStyle, Div, ElementId, InteractiveElement as _, IntoElement as _,
    MouseButton, ParentElement as _, Role, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, Window, div,
};

use super::{AppIconSize, activates_button, app_icon, icon_control};
use crate::app::ui::{assets::AppIcon, theme::THEME};

pub(crate) fn disclosure_button(
    id: impl Into<ElementId>,
    expanded: bool,
    label: impl Into<SharedString>,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let label = label.into();
    icon_control(id, disclosure_action_label(expanded, &label))
        .aria_expanded(expanded)
        .text_color(THEME.colors.muted)
        .hover(|control| control.bg(THEME.colors.hover))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_press(window, cx);
        })
        .child(app_icon(
            if expanded {
                AppIcon::CaretDown
            } else {
                AppIcon::CaretRight
            },
            AppIconSize::Control,
        ))
        .into_any_element()
}

pub(crate) fn disclosure_detail() -> Div {
    div()
        .ml(THEME.icons.control + THEME.space.xs)
        .mt(THEME.space.xs)
}

type DisclosureHandler = Rc<dyn Fn(&mut Window, &mut App)>;

pub(crate) fn disclosure_title_row(
    id: impl Into<ElementId>,
    expanded: bool,
    expandable: bool,
    label: impl Into<SharedString>,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let row = div()
        .id(id)
        .w_full()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .rounded(THEME.radius);
    if !expandable {
        return row;
    }

    let label = label.into();
    let on_press: DisclosureHandler = Rc::new(on_press);
    let click = Rc::clone(&on_press);
    row.role(Role::Button)
        .child(
            div()
                .flex_none()
                .text_size(THEME.type_scale.body_small)
                .text_color(THEME.colors.muted)
                .child(if expanded { "Hide details" } else { "Details" }),
        )
        .aria_label(disclosure_action_label(expanded, &label))
        .aria_expanded(expanded)
        .tab_index(0)
        .cursor(CursorStyle::PointingHand)
        .hover(|row| row.bg(THEME.colors.hover))
        .focus_visible(|row| row.border(THEME.border).border_color(THEME.colors.accent))
        .on_mouse_down(MouseButton::Left, super::preserve_pointer_focus)
        .on_click(move |_, window, cx| click(window, cx))
        .on_key_down(move |event, window, cx| {
            if activates_button(event) {
                cx.stop_propagation();
                on_press(window, cx);
            }
        })
}

fn disclosure_action_label(expanded: bool, label: &str) -> String {
    format!("{} {label}", if expanded { "Collapse" } else { "Expand" })
}
