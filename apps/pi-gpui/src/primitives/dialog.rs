use crate::theme::THEME;
use gpui::{
    App, Div, ElementId, InteractiveElement as _, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

pub(crate) fn dialog_backdrop(
    id: impl Into<ElementId>,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p(THEME.space.md)
        .bg(THEME.colors.backdrop)
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| on_dismiss(window, cx))
}

pub(crate) fn dialog_surface(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Dialog)
        .aria_label(label)
        .tab_group()
        .w(THEME.layout.dialog_width)
        .max_w_full()
        .max_h(THEME.layout.dialog_max_height)
        .overflow_y_scroll()
        .rounded(px(12.0))
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.panel)
        .on_click(|_, _, cx| cx.stop_propagation())
}
