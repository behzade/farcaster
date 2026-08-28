use crate::theme::THEME;
use gpui::{
    App, Div, FocusHandle, InteractiveElement as _, ParentElement as _, Role, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Window, div,
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
        .w(THEME.layout.dialog_width)
        .max_w_full()
        .max_h(THEME.layout.dialog_max_height)
        .overflow_y_scroll()
        .rounded(THEME.radius)
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.panel)
        .on_click(|_, _, cx| cx.stop_propagation())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, point, px, size};
    use std::{cell::Cell, rc::Rc};

    #[gpui::test]
    fn backdrop_prevents_background_hover(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        let background_hovered = Rc::new(Cell::new(false));
        let hover_state = background_hovered.clone();

        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(100.0), px(100.0)),
            move |_, _| {
                div()
                    .relative()
                    .size_full()
                    .child(
                        div()
                            .id("background-hover-target")
                            .absolute()
                            .inset_0()
                            .on_hover(move |hovered, _, _| hover_state.set(*hovered)),
                    )
                    .child(dialog_backdrop("test-backdrop", |_, _| {}))
            },
        );
        cx.simulate_mouse_move(point(px(25.0), px(25.0)), None, Modifiers::default());

        assert!(!background_hovered.get());
    }
}
