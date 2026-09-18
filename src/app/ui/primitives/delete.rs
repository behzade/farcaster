use gpui::{
    App, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    prelude::FluentBuilder as _,
};

use crate::app::ui::{assets::AppIcon, theme::theme};

use super::icon::{AppIconSize, app_icon, icon_control};

/// The one trash control in the app. Rows that hide it until hover pass the
/// group they belong to so the affordance is identical everywhere.
#[derive(IntoElement)]
pub(crate) struct DeleteButton {
    id: ElementId,
    label: SharedString,
    reveal_group: Option<SharedString>,
    action: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
}

impl DeleteButton {
    pub(crate) fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            reveal_group: None,
            action: None,
        }
    }

    pub(crate) fn reveal_on(mut self, group: impl Into<SharedString>) -> Self {
        self.reveal_group = Some(group.into());
        self
    }

    pub(crate) fn on_delete(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.action = Some(Box::new(action));
        self
    }
}

impl RenderOnce for DeleteButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self {
            id,
            label,
            reveal_group,
            action,
        } = self;
        icon_control(id, label)
            .opacity(if reveal_group.is_some() { 0.0 } else { 1.0 })
            .when_some(reveal_group, |button, group| {
                button
                    .group_hover(group, |button| button.opacity(1.0))
                    .focus(|button| button.opacity(1.0))
            })
            .text_color(theme().colors.danger)
            .hover(|button| button.bg(theme().colors.highlight))
            .child(app_icon(AppIcon::Trash, AppIconSize::Control))
            .when_some(action, |button, action| {
                button.on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    action(window, cx);
                })
            })
    }
}
