use gpui::{
    IntoElement, ParentElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};

use super::super::PiApp;
use crate::{
    primitives::{ButtonTone, button},
    theme::THEME,
};

impl PiApp {
    pub(super) fn render_navigation(
        &self,
        show_sessions: bool,
        work_active: bool,
        entity: WeakEntity<Self>,
    ) -> impl IntoElement {
        let sessions_entity = entity.clone();
        let chat_entity = entity.clone();
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
                        let _ = sessions_entity
                            .update(cx, |this, cx| this.open_sessions_sheet(window, cx));
                    },
                ))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(THEME.space.xs)
                    .child(button(
                        "show-chat-surface",
                        "Chat",
                        if work_active {
                            ButtonTone::Quiet
                        } else {
                            ButtonTone::Neutral
                        },
                        true,
                        move |window, cx| {
                            let _ = chat_entity
                                .update(cx, |this, cx| this.show_chat_surface(window, cx));
                        },
                    ))
                    .child({
                        let workgraph = entity.clone();
                        button(
                            "show-workgraph-surface",
                            "Work",
                            if work_active {
                                ButtonTone::Neutral
                            } else {
                                ButtonTone::Quiet
                            },
                            true,
                            move |window, cx| {
                                let _ = workgraph.update(cx, |this, cx| {
                                    this.open_workgraph_surface(window, cx);
                                });
                            },
                        )
                    })
                    .when(!work_active, |navigation| {
                        navigation.child(button(
                            "open-run",
                            "Session details",
                            ButtonTone::Quiet,
                            true,
                            move |window, cx| {
                                let _ =
                                    entity.update(cx, |this, cx| this.open_run_sheet(window, cx));
                            },
                        ))
                    }),
            )
    }
}
