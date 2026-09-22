use gpui::{AnyElement, IntoElement, ParentElement as _, Pixels, Styled as _, div};

use crate::app::ui::theme::theme;

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
