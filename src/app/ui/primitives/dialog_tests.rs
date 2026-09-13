use super::*;
use gpui::{Modifiers, TestAppContext, point, px, size};
use std::{cell::Cell, rc::Rc};

struct ConfirmationHarness {
    focus: FocusHandle,
    confirmed: Rc<Cell<usize>>,
    dismissed: Rc<Cell<usize>>,
    background: Rc<Cell<usize>>,
}

impl gpui::Render for ConfirmationHarness {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use crate::app::ui::primitives::{ButtonTone, button};
        let confirm = self.confirmed.clone();
        let dismiss = self.dismissed.clone();
        let background_keys = self.background.clone();
        let background_action = self.background.clone();
        div()
            .size_full()
            .on_key_down(move |_, _, _| background_keys.set(background_keys.get() + 1))
            .on_action(move |_: &crate::app::DismissSurface, _, _| {
                background_action.set(background_action.get() + 1);
            })
            .child(confirmation_modal(
                "test-confirmation",
                "Confirm?",
                &self.focus,
                crate::app::OVERLAY_KEY_CONTEXT,
                move |_, _| dismiss.set(dismiss.get() + 1),
                move |_, _| confirm.set(confirm.get() + 1),
                |surface| {
                    surface.child(button(
                        "cancel",
                        "Cancel",
                        ButtonTone::Neutral,
                        true,
                        |_, _| panic!("Enter must confirm, even when Cancel has focus"),
                    ))
                },
            ))
    }
}

#[gpui::test]
fn enter_confirms_and_escape_cancels_regardless_of_button_focus(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    cx.update(|cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "escape",
            crate::app::DismissSurface,
            Some(crate::app::OVERLAY_KEY_CONTEXT),
        )]);
    });
    let confirmed = Rc::new(Cell::new(0));
    let dismissed = Rc::new(Cell::new(0));
    let background = Rc::new(Cell::new(0));
    let (_, cx) = cx.add_window_view(|window, cx| {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        ConfirmationHarness {
            focus,
            confirmed: confirmed.clone(),
            dismissed: dismissed.clone(),
            background: background.clone(),
        }
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    for count in 1..=2 {
        cx.simulate_keystrokes("enter");
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("enter").expect("test key"),
        });
        assert_eq!((confirmed.get(), dismissed.get()), (count, count - 1));
        cx.simulate_keystrokes("escape");
        assert_eq!(
            (confirmed.get(), dismissed.get(), background.get()),
            (count, count, 0)
        );
        cx.simulate_keystrokes("tab");
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
}

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
