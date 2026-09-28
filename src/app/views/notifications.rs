use gpui::{
    AnyElement, App, ElementId, FontWeight, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Role, StatefulInteractiveElement as _, Styled as _, WeakEntity, Window,
    accesskit, div,
};

use super::FarcasterApp;
use crate::{
    app::ui::{
        primitives::{FeedbackTone, activates_button, feedback},
        theme::theme,
    },
    protocol::NotifyTone,
};

impl FarcasterApp {
    pub(in crate::app::views) fn render_notification(
        &self,
        id: impl Into<ElementId>,
        tag: &str,
        message: &str,
        tone: NotifyTone,
        entity: WeakEntity<Self>,
    ) -> AnyElement {
        let tone = match tone {
            NotifyTone::Error => FeedbackTone::Error,
            NotifyTone::Warning => FeedbackTone::Warning,
            NotifyTone::Info => FeedbackTone::Info,
        };
        let Some((path, project)) = self.activity.system_notification_targets.get(tag).cloned()
        else {
            return feedback(id, message.to_owned(), tone);
        };
        let open = move |window: &mut Window, cx: &mut App| {
            let _ = entity.update(cx, |app, cx| {
                app.select_session_and_focus(path.clone(), project.clone(), window, cx);
            });
        };
        let click = open.clone();
        let (title, preview) = message.split_once('\n').unwrap_or((message, ""));
        let accessible = message.to_owned();
        div()
            .id(id)
            .debug_selector(|| "session-notification".into())
            .role(Role::Button)
            .aria_label(format!("Open session: {message}"))
            .a11y_synthetic_children(move |builder| {
                builder.parent_node().set_live(accesskit::Live::Polite);
                builder.parent_node().set_value(accessible.as_str());
            })
            .tab_index(0)
            .cursor_pointer()
            .flex()
            .flex_col()
            .gap(theme().size(2.0))
            .px(theme().size(18.0))
            .py(theme().size(12.0))
            .rounded(theme().radius)
            .bg(theme().colors.panel)
            .border(theme().border)
            .border_color(theme().colors.border)
            .text_size(theme().type_scale.body)
            .text_color(theme().colors.text)
            .shadow_md()
            .hover(|style| style.bg(theme().colors.highlight))
            .focus_visible(|style| style.border_color(theme().colors.accent))
            .child(
                div()
                    .min_w_0()
                    .font_weight(FontWeight::SEMIBOLD)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(title.to_owned()),
            )
            .child(
                div()
                    .min_w_0()
                    .line_clamp(2)
                    .overflow_hidden()
                    .child(preview.to_owned()),
            )
            .on_click(move |_, window, cx| click(window, cx))
            .on_key_down(move |event, window, cx| {
                if activates_button(event) {
                    cx.stop_propagation();
                    open(window, cx);
                }
            })
            .into_any_element()
    }
}
