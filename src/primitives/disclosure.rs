use std::rc::Rc;

use gpui::{
    AnyElement, App, CursorStyle, Div, ElementId, InteractiveElement as _, IntoElement as _,
    MouseButton, ParentElement as _, Role, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};
use gpui_base::GlobalState;

use super::{AppIconSize, activates_button, app_icon, icon_control};
use crate::{assets::AppIcon, theme::THEME};

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
    key: usize,
    expanded: bool,
    expandable: bool,
    label: impl Into<SharedString>,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let group = SharedString::from(format!("disclosure-{key}"));
    let row = div()
        .id(id)
        .group(group.clone())
        .w_full()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .rounded(THEME.radius)
        .child(disclosure_gutter(expanded, expandable, group));
    if !expandable {
        return row;
    }

    let label = label.into();
    let on_press: DisclosureHandler = Rc::new(on_press);
    let click = Rc::clone(&on_press);
    row.role(Role::Button)
        .aria_label(disclosure_action_label(expanded, &label))
        .aria_expanded(expanded)
        .tab_index(0)
        .cursor(CursorStyle::PointingHand)
        .hover(|row| row.bg(THEME.colors.hover))
        .focus_visible(|row| row.border(THEME.border).border_color(THEME.colors.accent))
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            GlobalState::suppress_text_selection(cx);
        })
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

fn disclosure_gutter(expanded: bool, expandable: bool, group: SharedString) -> AnyElement {
    div()
        .w(THEME.icons.control)
        .h(THEME.icons.control)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .text_color(THEME.colors.subtle)
        .when(expandable, |slot| {
            slot.when(!expanded, |slot| {
                slot.opacity(0.0)
                    .group_hover(group, |slot| slot.opacity(1.0))
            })
            .child(app_icon(AppIcon::CaretDown, AppIconSize::Control))
        })
        .into_any_element()
}
