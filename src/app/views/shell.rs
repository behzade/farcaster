use gpui::{
    IntoElement, ParentElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};

use super::super::FarcasterApp;
use crate::{
    app::ui::primitives::{ButtonTone, button},
    app::ui::theme::THEME,
};

impl FarcasterApp {
    pub(super) fn render_chat_navigation(
        &self,
        show_sessions: bool,
        entity: WeakEntity<Self>,
    ) -> impl IntoElement {
        let sessions = entity.clone();
        let work = entity.clone();
        div()
            .h(px(40.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px(THEME.space.sm)
            .border_b(THEME.border)
            .border_color(THEME.colors.border)
            .bg(THEME.colors.panel)
            .when(show_sessions, |navigation| {
                navigation.child(button(
                    "open-sessions",
                    "Sessions",
                    ButtonTone::Quiet,
                    true,
                    move |window, cx| {
                        let _ = sessions.update(cx, |this, cx| {
                            this.open_sessions_sheet(window, cx);
                        });
                    },
                ))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(THEME.space.xs)
                    .child(button(
                        "open-project-work",
                        "Project work",
                        ButtonTone::Quiet,
                        true,
                        move |window, cx| {
                            let _ = work.update(cx, |this, cx| {
                                this.open_workgraph_surface(window, cx);
                            });
                        },
                    ))
                    .child(button(
                        "open-run",
                        "Session details",
                        ButtonTone::Quiet,
                        true,
                        move |window, cx| {
                            let _ = entity.update(cx, |this, cx| this.open_run_sheet(window, cx));
                        },
                    )),
            )
    }
}
