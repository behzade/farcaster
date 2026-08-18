use crate::theme::THEME;
use gpui::{
    App, Div, FocusHandle, InteractiveElement as _, ParentElement as _, Role, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::FocusTrapElement as _;

pub(crate) fn modal(
    id: &'static str,
    label: impl Into<SharedString>,
    focus: &FocusHandle,
    key_context: &'static str,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
    configure: impl FnOnce(Stateful<Div>) -> Stateful<Div>,
) -> Stateful<Div> {
    let surface = configure(dialog_surface(format!("{id}-surface"), label))
        .track_focus(focus)
        .key_context(key_context)
        .focus_trap(format!("{id}-focus-trap"), focus);
    dialog_backdrop(format!("{id}-backdrop"), on_dismiss).child(surface)
}

fn dialog_backdrop(
    id: impl Into<gpui::ElementId>,
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

fn dialog_surface(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
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
