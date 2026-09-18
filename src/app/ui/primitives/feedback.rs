use crate::app::ui::theme::theme;
use gpui::{
    AnyElement, ElementId, InteractiveElement as _, IntoElement as _, ParentElement as _, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, accesskit, div,
};

#[derive(Clone, Copy)]
pub(crate) enum FeedbackTone {
    Error,
    Warning,
    Info,
}

pub(crate) fn feedback(
    id: impl Into<ElementId>,
    message: impl Into<SharedString>,
    tone: FeedbackTone,
) -> AnyElement {
    let message = message.into();
    let accessible = message.clone();
    let (role, live, color) = match tone {
        FeedbackTone::Error => (
            Role::Alert,
            accesskit::Live::Assertive,
            theme().colors.error,
        ),
        FeedbackTone::Warning => (
            Role::Status,
            accesskit::Live::Assertive,
            theme().colors.warning,
        ),
        FeedbackTone::Info => (Role::Status, accesskit::Live::Polite, theme().colors.accent),
    };
    div()
        .id(id)
        .role(role)
        .a11y_synthetic_children(move |builder| {
            builder.parent_node().set_live(live);
            builder.parent_node().set_value(accessible.as_ref());
        })
        .rounded(theme().radius)
        .bg(theme().colors.panel)
        .border(theme().border)
        .border_color(color)
        .px(theme().space.sm)
        .py(theme().space.xs)
        .text_size(theme().type_scale.caption)
        .text_color(color)
        .child(message)
        .into_any_element()
}
