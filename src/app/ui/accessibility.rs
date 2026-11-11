use gpui::{
    AnyElement, App, ElementId, Entity, EventEmitter, IntoElement as _, Render, StyleRefinement,
    Styled as _, Window,
};
use gpui_component::input::InputEvent;

/// Occluded macOS windows stop receiving display frames. Publish an updated
/// accessibility tree after an action without waiting for the window to be raised.
pub(crate) fn refresh_hidden_accessibility(window: &mut Window, cx: &mut App) {
    if window.is_a11y_active() && !window.visibility().is_visible() {
        defer_accessibility_draw(window, cx);
    }
}

fn defer_accessibility_draw(window: &mut Window, cx: &mut App) {
    // The action may still hold the root view or input entity. Drawing inside
    // that lease would borrow it twice, so wait until the effect cycle ends.
    window.defer(cx, |window, cx| {
        window.draw(cx).clear(cx);
        #[cfg(target_os = "macos")]
        crate::app::views::native_overlay::native::NativeHost::request_background_frame(window);
    });
}

pub(crate) fn observe_accessible_input<T: EventEmitter<InputEvent> + 'static>(
    input: &Entity<T>,
    window: &mut Window,
    cx: &mut App,
) {
    window.use_keyed_state(
        ElementId::NamedInteger("accessible-input-events".into(), input.entity_id().as_u64()),
        cx,
        |window, cx| {
            window.subscribe(input, cx, |_, event, window, cx| {
                if matches!(
                    event,
                    InputEvent::Change | InputEvent::Focus | InputEvent::Blur
                ) {
                    refresh_hidden_accessibility(window, cx);
                }
            })
        },
    );
}

/// GPUI's cached prepaint replays drawing and input state, but not accessibility nodes.
/// Visit the subtree on every accessible frame so its controls stay in the AX tree.
pub(crate) fn cached_panel<V: Render>(view: Entity<V>, accessibility_active: bool) -> AnyElement {
    if accessibility_active {
        view.into_any_element()
    } else {
        view.cached(StyleRefinement::default().size_full())
            .into_any_element()
    }
}

#[cfg(test)]
#[path = "accessibility_tests.rs"]
mod tests;
