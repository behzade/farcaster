use std::rc::Rc;

use gpui::{
    AnyElement, App, CursorStyle, Div, ElementId, InteractiveElement as _, IntoElement as _,
    MouseButton, ParentElement as _, Role, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, Window, div,
};

use super::{AppIconSize, activates_button, app_icon, icon_control};
use crate::app::ui::{assets::AppIcon, theme::theme};

pub(crate) fn disclosure_button(
    id: impl Into<ElementId>,
    expanded: bool,
    label: impl Into<SharedString>,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let label = label.into();
    icon_control(id, disclosure_action_label(expanded, &label))
        .aria_expanded(expanded)
        .text_color(theme().colors.muted)
        .hover(|control| control.bg(theme().colors.highlight))
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
        .ml(theme().icons.control + theme().space.xs)
        .mt(theme().space.xs)
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
        .gap(theme().space.xs)
        .rounded(theme().radius);
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
                .text_size(theme().type_scale.body_small)
                .text_color(theme().colors.muted)
                .child(if expanded { "Hide details" } else { "Details" }),
        )
        .aria_label(disclosure_action_label(expanded, &label))
        .aria_expanded(expanded)
        .tab_index(0)
        .cursor(CursorStyle::PointingHand)
        .hover(|row| row.bg(theme().colors.highlight))
        .focus_visible(|row| {
            row.border(theme().border)
                .border_color(theme().colors.accent)
        })
        .on_mouse_down(MouseButton::Left, super::preserve_pointer_focus)
        .on_click(move |_, window, cx| click(window, cx))
        .on_key_down(move |event, window, cx| {
            if activates_button(event) {
                cx.stop_propagation();
                on_press(window, cx);
            }
        })
}

/// Compact folder disclosure shared by repository and review trees.
pub(crate) fn tree_folder_row(
    id: impl Into<ElementId>,
    label: String,
    depth: usize,
    expanded: bool,
    enabled: bool,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let row = div()
        .id(id)
        .role(Role::Button)
        .aria_label(disclosure_action_label(expanded, &label))
        .aria_expanded(expanded)
        .w_full()
        .min_w_0()
        .h(theme().size(24.0))
        .pl(gpui::px(depth as f32 * 12.0 + 4.0))
        .pr(theme().space.xs)
        .flex()
        .items_center()
        .gap(theme().space.xs)
        .rounded(theme().radius)
        .text_size(theme().type_scale.caption)
        .text_color(theme().colors.muted)
        .child(div().w(theme().size(14.0)).flex_none().child(app_icon(
            if expanded {
                AppIcon::CaretDown
            } else {
                AppIcon::CaretRight
            },
            AppIconSize::Inline,
        )))
        .child(div().min_w_0().flex_1().text_ellipsis().child(label));
    if !enabled {
        return row;
    }
    let on_press: DisclosureHandler = Rc::new(on_press);
    let click = Rc::clone(&on_press);
    row.tab_index(0)
        .cursor_pointer()
        .hover(|row| row.bg(theme().colors.highlight))
        .focus_visible(|row| row.bg(theme().colors.highlight))
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
