//! Dialog textarea behavior: Enter submits, Shift+Enter inserts a newline.

use gpui::{
    AppContext as _, Context, Div, Entity, InteractiveElement as _, ParentElement as _,
    Styled as _, Subscription, Window, div,
};
use gpui_component::input::{Enter, InputEvent, Textarea, TextareaState};

pub(crate) fn create_submit_textarea<T: 'static>(
    window: &mut Window,
    cx: &mut Context<T>,
    configure: impl FnOnce(TextareaState) -> TextareaState,
    on_submit: impl Fn(&mut T, &mut Window, &mut Context<T>) + 'static,
) -> (Entity<TextareaState>, Subscription) {
    let input = cx.new(|cx| configure(TextareaState::new(window, cx)).submit_on_enter(true));
    let subscription = cx.subscribe_in(&input, window, move |this, _, event, window, cx| {
        if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
            on_submit(this, window, cx);
        }
        cx.notify();
    });
    (input, subscription)
}

pub(crate) fn submit_textarea(input: Textarea) -> Div {
    // Consume Enter after PressEnter is emitted, preventing a fallback newline.
    div()
        .w_full()
        .on_action(|_: &Enter, _, _| {})
        .child(input.w_full())
}

#[cfg(test)]
#[path = "textarea_tests.rs"]
mod tests;
