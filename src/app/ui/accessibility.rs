use gpui::{AnyElement, Entity, IntoElement as _, Render, StyleRefinement, Styled as _};

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
