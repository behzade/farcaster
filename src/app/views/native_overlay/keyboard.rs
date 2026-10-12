use std::ops::Range;

use gpui::{
    AnyElement, AnyWindowHandle, App, Bounds, ClipboardItem, ElementInputHandler, FocusHandle,
    InputHandler, IntoElement as _, KeyDownEvent, Keystroke, Modifiers, Pixels, PlatformInput,
    Point, UTF16Selection, Window, canvas, div,
};
use gpui_component::{WindowExt as _, input::AnyInputState};

use super::native::OverlayInput;

struct Relay {
    owner: AnyWindowHandle,
    popup: AnyWindowHandle,
    offset: Point<Pixels>,
    enabled: bool,
}

#[derive(Default)]
struct Relays(Vec<Relay>);
impl gpui::Global for Relays {}

pub(super) fn register(
    owner: AnyWindowHandle,
    popup: AnyWindowHandle,
    offset: Point<Pixels>,
    input: OverlayInput,
    cx: &mut App,
) {
    if !cx.has_global::<Relays>() {
        cx.set_global(Relays::default());
    }
    cx.global_mut::<Relays>().0.push(Relay {
        owner,
        popup,
        offset,
        enabled: input == OverlayInput::Modal,
    });
    refresh_owner(owner, cx);
}

pub(super) fn unregister(popup: AnyWindowHandle, cx: &mut App) {
    let owner = cx.try_global::<Relays>().and_then(|relays| {
        relays
            .0
            .iter()
            .find(|relay| relay.popup == popup)
            .map(|relay| relay.owner)
    });
    if let Some(owner) = owner {
        cx.global_mut::<Relays>()
            .0
            .retain(|relay| relay.popup != popup);
        refresh_owner(owner, cx);
    }
}

fn refresh_owner(owner: AnyWindowHandle, cx: &mut App) {
    cx.defer(move |cx| {
        let _ = owner.update(cx, |_, window, _| window.refresh());
    });
}

fn target(window: &Window, cx: &App) -> Option<(AnyWindowHandle, Point<Pixels>)> {
    let live = cx.windows();
    cx.try_global::<Relays>()?
        .0
        .iter()
        .rev()
        .find(|relay| {
            relay.owner == window.window_handle() && relay.enabled && live.contains(&relay.popup)
        })
        .map(|relay| (relay.popup, relay.offset))
}

// Run before the owner's shortcuts or native terminal see the keystroke.
// Preserve key_char from XKB rather than synthesizing text from the key name.
pub(in crate::app) fn relay_keystroke(key: &Keystroke, window: &Window, cx: &mut App) -> bool {
    let Some((popup, _)) = target(window, cx) else {
        return false;
    };
    let key = key.clone();
    // Menu callbacks may update their owner window. Release the owner's event
    // borrow before dispatching in the popup.
    cx.defer(move |cx| {
        let _ = popup.update(cx, |_, window, cx| {
            let result = window.dispatch_event(
                PlatformInput::KeyDown(KeyDownEvent {
                    keystroke: key.clone(),
                    is_held: false,
                    prefer_character_input: false,
                }),
                cx,
            );
            if result.propagate
                && key.modifiers.is_subset_of(&Modifiers::shift())
                && let Some(text) = key.key_char.as_deref()
            {
                // Match Wayland's normal post-key-dispatch text insertion.
                if let Some(mut input) = focused_input(window, cx) {
                    input.replace_text_in_range(None, text, window, cx);
                }
            }
        });
    });
    cx.stop_propagation();
    true
}

pub(in crate::app) fn input_proxy(window: &Window, fallback: &FocusHandle, cx: &App) -> AnyElement {
    let Some((popup, offset)) = target(window, cx) else {
        return div().into_any_element();
    };
    let fallback = fallback.clone();
    canvas(
        move |_, window, cx| {
            if window.focused(cx).is_none() {
                fallback.focus(window, cx);
            }
        },
        move |_, _, window, cx| {
            // Replace the native terminal's input handler while a modal owns input.
            // The compositor keeps keyboard focus on this owner, including IME.
            if let Some(focus) = window.focused(cx) {
                window.handle_input(&focus, OwnerInput { popup, offset }, cx);
            }
        },
    )
    .into_any_element()
}

fn focused_input(window: &mut Window, cx: &mut App) -> Option<Box<dyn InputHandler>> {
    macro_rules! handler {
        ($state:expr) => {{
            let bounds = $state.read(cx).input_bounds();
            Box::new(ElementInputHandler::new(bounds, $state)) as Box<dyn InputHandler>
        }};
    }
    Some(match window.focused_input(cx)? {
        AnyInputState::Input(state) => handler!(state),
        AnyInputState::Textarea(state) => handler!(state),
        AnyInputState::Editor(state) => handler!(state),
        AnyInputState::Otp(_) => return None,
    })
}

struct OwnerInput {
    popup: AnyWindowHandle,
    offset: Point<Pixels>,
}

impl OwnerInput {
    fn with_input<R>(
        &self,
        cx: &mut App,
        run: impl FnOnce(&mut dyn InputHandler, &mut Window, &mut App) -> R,
    ) -> Option<R> {
        self.popup
            .update(cx, |_, window, cx| {
                let mut input = focused_input(window, cx)?;
                Some(run(&mut *input, window, cx))
            })
            .ok()
            .flatten()
    }
}

// Forward text operations so composition, selection, paste and
// UTF-16 ranges use the popup's actual focused input and window.
macro_rules! forward {
    ($name:ident($($arg:ident: $ty:ty),*) -> $result:ty, $empty:expr) => {
        fn $name(&mut self, $($arg: $ty,)* _window: &mut Window, cx: &mut App) -> $result {
            self.with_input(cx, |input, window, cx| input.$name($($arg,)* window, cx)).unwrap_or($empty)
        }
    };
}

impl InputHandler for OwnerInput {
    // App inputs use the default text configuration. Querying popup state in
    // this per-frame hook would make owner and popup redraw each other.
    forward!(selected_text_range(ignore_disabled_input: bool) -> Option<UTF16Selection>, None);
    forward!(marked_text_range() -> Option<Range<usize>>, None);
    forward!(text_for_range(range: Range<usize>, adjusted: &mut Option<Range<usize>>) -> Option<String>, None);
    forward!(replace_text_in_range(range: Option<Range<usize>>, text: &str) -> (), ());
    forward!(replace_and_mark_text_in_range(range: Option<Range<usize>>, text: &str, selected: Option<Range<usize>>) -> (), ());
    forward!(unmark_text() -> (), ());
    forward!(paste(item: ClipboardItem) -> (), ());
    forward!(set_selected_text_range(range: Range<usize>) -> (), ());
    forward!(text_length_utf16() -> Option<usize>, None);
    forward!(text_input_editable_range() -> Option<Range<usize>>, None);

    fn accepts_text_input(&mut self, window: &mut Window, cx: &mut App) -> bool {
        // Enable the owner's IME session for the modal's lifetime. Its focused
        // input can change by a popup click without repainting the owner.
        target(window, cx).is_some_and(|(popup, _)| popup == self.popup)
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        let mut bounds = self.with_input(cx, |input, window, cx| {
            input.bounds_for_range(range, window, cx)
        })??;
        bounds.origin += self.offset;
        Some(bounds)
    }

    fn element_bounds(&mut self, _: &mut Window, cx: &mut App) -> Option<Bounds<Pixels>> {
        let mut bounds =
            self.with_input(cx, |input, window, cx| input.element_bounds(window, cx))??;
        bounds.origin += self.offset;
        Some(bounds)
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<usize> {
        self.with_input(cx, |input, window, cx| {
            input.character_index_for_point(point - self.offset, window, cx)
        })?
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
#[path = "../native_keyboard_tests.rs"]
mod tests;
