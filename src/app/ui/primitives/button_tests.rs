use super::*;

fn key_event(key: &'static str, is_held: bool) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: gpui::Keystroke::parse(key).expect("test keystroke"),
        is_held,
        prefer_character_input: false,
    }
}

#[test]
fn icon_buttons_activate_on_unmodified_enter_or_space_once() {
    assert!(activates_button(&key_event("enter", false)));
    assert!(activates_button(&key_event("space", false)));
    assert!(!activates_button(&key_event("enter", true)));
    assert!(!activates_button(&key_event("cmd-enter", false)));
    assert!(!activates_button(&key_event("a", false)));
}

#[gpui::test]
fn accessibility_press_keeps_the_users_active_window(cx: &mut gpui::TestAppContext) {
    use gpui::{Context, Render};

    struct Control;
    impl Render for Control {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    let background = cx
        .add_window_view(|_, _| Control)
        .1
        .update(|window, _| window.window_handle());
    let foreground = cx
        .add_window_view(|_, _| Control)
        .1
        .update(|window, _| window.window_handle());
    foreground
        .update(cx, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    let active = cx.read(|cx| cx.active_window());
    assert!(active.is_some());
    let presses = Rc::new(Cell::new(0));
    let pressed = presses.clone();
    background
        .update(cx, |_, window, cx| {
            assert!(!window.is_window_active());
            press_accessibly(&move |_, _| pressed.set(pressed.get() + 1), window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(presses.get(), 1, "the background control must still work");
    assert_eq!(cx.read(|cx| cx.active_window()), active);
    background
        .update(cx, |_, window, _| assert!(!window.is_window_active()))
        .unwrap();
}
