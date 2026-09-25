use crate::app::ui::theme::theme;
use gpui::{
    App, Div, FocusHandle, InteractiveElement as _, ParentElement as _, Role, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Window, div,
};
use gpui_component::FocusTrapElement as _;

pub(crate) fn confirmation_modal(
    id: &'static str,
    label: impl Into<SharedString>,
    focus: &FocusHandle,
    key_context: &'static str,
    on_dismiss: impl Fn(&mut Window, &mut App) + Clone + 'static,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
    configure: impl FnOnce(Stateful<Div>) -> Stateful<Div>,
) -> Stateful<Div> {
    modal(
        id,
        label,
        focus,
        key_context,
        on_dismiss.clone(),
        |surface| {
            configure(surface)
                .on_action(move |_: &crate::app::DismissSurface, window, cx| {
                    cx.stop_propagation();
                    on_dismiss(window, cx);
                })
                .capture_key_down(move |event, window, cx| {
                    if event.keystroke.key == "enter"
                        && event.keystroke.modifiers == gpui::Modifiers::default()
                    {
                        cx.stop_propagation();
                        window.prevent_default();
                        if !event.is_held {
                            on_confirm(window, cx);
                        }
                    }
                })
        },
    )
}

pub(crate) fn modal(
    id: &'static str,
    label: impl Into<SharedString>,
    focus: &FocusHandle,
    key_context: &'static str,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
    configure: impl FnOnce(Stateful<Div>) -> Stateful<Div>,
) -> Stateful<Div> {
    let traversal_scope = focus.clone();
    let surface = configure(dialog_surface(format!("{id}-surface"), label))
        .track_focus(focus)
        .key_context(key_context)
        .on_key_down(move |event, window, cx| {
            crate::app::ui::focus::traverse_tab(event, Some(&traversal_scope), window, cx);
        })
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
        .p(theme().space.md)
        .bg(theme().colors.backdrop)
        .occlude()
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .id("modal-dismiss-layer")
                .absolute()
                .inset_0()
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    on_dismiss(window, cx);
                }),
        )
}

fn dialog_surface(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Dialog)
        .aria_label(label)
        .tab_group()
        .w(theme().layout.dialog_width)
        .max_w_full()
        .max_h(theme().layout.dialog_max_height)
        .overflow_y_scroll()
        .rounded(theme().radius)
        .border(theme().border)
        .border_color(theme().colors.border)
        .bg(theme().colors.panel)
        .on_click(|_, _, cx| cx.stop_propagation())
}

#[cfg(test)]
#[path = "dialog_tests.rs"]
mod tests;
