use gpui::{
    InteractiveElement as _, IntoElement as _, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, div,
};

use crate::app::ui::theme::{MONO_FONT_FAMILY, theme};

pub(super) fn render(
    placement: &str,
    widgets: &std::collections::BTreeMap<String, Vec<String>>,
) -> Option<gpui::AnyElement> {
    if widgets.is_empty() {
        return None;
    }
    Some(
        div()
            .id(format!("widgets-{placement}"))
            .max_h(theme().layout.tool_max_height)
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(theme().space.xs)
            .children(widgets.iter().map(|(key, lines)| {
                div()
                    .id(format!("widget-{placement}-{key}"))
                    .px(theme().space.sm)
                    .py(theme().space.xs)
                    .bg(theme().colors.surface)
                    .font_family(MONO_FONT_FAMILY)
                    .text_size(theme().type_scale.caption)
                    .children(lines.iter().cloned().map(|line| div().child(line)))
            }))
            .into_any_element(),
    )
}
