//! Shared anchors for rows and sections. Containers own their outer inset;
//! children reserve slots instead of moving text when controls appear.
use gpui::{Div, Pixels, Styled as _, div};

use super::theme::theme;

pub(crate) fn sidebar_inset() -> Pixels {
    theme().size(12.0)
}

pub(crate) fn dialog_inset() -> Pixels {
    theme().space.md
}

pub(crate) fn form_inset() -> Pixels {
    theme().size(24.0)
}

pub(crate) fn transcript_inset() -> Pixels {
    theme().size(18.0)
}

pub(crate) fn tree_indent(depth: usize) -> Pixels {
    theme().size(12.0) * depth as f32
}

pub(crate) fn row() -> Div {
    div()
        .w_full()
        .min_w_0()
        .flex()
        .items_center()
        .gap(theme().space.xs)
}

pub(crate) fn slot(width: Pixels) -> Div {
    div()
        .w(width)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
}

pub(crate) fn icon_slot() -> Div {
    slot(theme().icons.control).h(theme().type_scale.line_body)
}

pub(crate) fn detail_indent() -> Pixels {
    theme().icons.control + theme().space.xs
}
