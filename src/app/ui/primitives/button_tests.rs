use super::*;

fn key_event(key: &str, is_held: bool) -> KeyDownEvent {
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
