use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
};

use crate::app::{
    FarcasterApp,
    ui::{
        primitives::{ButtonTone, button},
        theme::theme,
    },
};

impl FarcasterApp {
    pub(in crate::app) fn render_code_task_notice(
        &self,
        entity: WeakEntity<Self>,
    ) -> Option<AnyElement> {
        let message = self.workspace.code_tasks.notice_message()?;
        let dismiss = entity.clone();
        Some(
            div()
                .id("code-task-notice")
                .role(gpui::Role::Status)
                .a11y_synthetic_children(move |builder| {
                    builder
                        .parent_node()
                        .set_live(gpui::accesskit::Live::Polite);
                    builder.parent_node().set_value(message);
                })
                .rounded(theme().radius)
                .bg(theme().colors.panel)
                .border(theme().border)
                .border_color(theme().colors.accent)
                .p(theme().space.sm)
                .flex()
                .items_center()
                .gap(theme().space.sm)
                .text_size(theme().type_scale.caption)
                .text_color(theme().colors.text)
                .child(message)
                .child(button(
                    "open-code-task",
                    "Open chat",
                    ButtonTone::Neutral,
                    true,
                    move |window, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.open_code_task_chat(window, cx);
                        });
                    },
                ))
                .child(button(
                    "dismiss-code-task",
                    "Dismiss",
                    ButtonTone::Neutral,
                    true,
                    move |_, cx| {
                        let _ = dismiss.update(cx, |this, cx| {
                            this.dismiss_code_task_notice(cx);
                        });
                    },
                ))
                .into_any_element(),
        )
    }
}
