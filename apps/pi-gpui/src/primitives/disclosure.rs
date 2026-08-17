use gpui::{
    AnyElement, App, ElementId, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::{primitives::icon_control, theme::THEME};

pub(crate) fn disclosure_indicator(expanded: bool) -> Icon {
    Icon::new(if expanded {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    })
    .with_size(THEME.icons.control)
}

pub(crate) fn disclosure_button(
    id: impl Into<ElementId>,
    expanded: bool,
    label: impl Into<SharedString>,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let label = label.into();
    let accessible_label = format!("{} {label}", if expanded { "Collapse" } else { "Expand" });
    icon_control(id, accessible_label)
        .aria_expanded(expanded)
        .text_color(THEME.colors.muted)
        .hover(|control| control.bg(THEME.colors.hover))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_press(window, cx);
        })
        .child(disclosure_indicator(expanded))
        .into_any_element()
}
