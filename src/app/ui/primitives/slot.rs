use gpui::{AnyElement, IntoElement, ParentElement as _, Pixels, Styled as _, div};

use crate::app::ui::theme::theme;

/// Every number in the UI — the chat age, the panel counts, the worker notice
/// tally — is drawn in this one slot: wide enough for the digits it can reach,
/// so the value never nudges what sits beside it, and wider only once it grows
/// past that.
pub(crate) fn number_slot(value: impl IntoElement, slot: Pixels) -> AnyElement {
    div()
        .min_w(slot)
        .flex_none()
        .flex()
        .justify_end()
        .whitespace_nowrap()
        .text_size(theme().type_scale.caption)
        .text_color(theme().colors.subtle)
        .child(value)
        .into_any_element()
}
