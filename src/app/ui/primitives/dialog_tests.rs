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
