use gpui::{
    FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};

use crate::app::{
    FarcasterApp, OVERLAY_KEY_CONTEXT,
    mcp_server::NoticeView,
    ui::{
        assets::AppIcon,
        primitives::{ButtonTone, icon_button, modal},
        theme::theme,
    },
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> impl IntoElement {
    let notices = app.worker_notices.snapshot(&app.project.path);
    let count = notices.len();
    let close = entity.clone();
    modal(
        "worker-notices",
        "Worker notices",
        &app.overlays.sheet_focus,
        OVERLAY_KEY_CONTEXT,
        move |window, cx| {
            let _ = close.update(cx, |app, cx| app.close_sheet(window, cx));
        },
        |surface| {
            let close = entity;
            surface
                .w(px(560.0))
                .max_w_full()
                .overflow_hidden()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px(theme().space.md)
                        .py(theme().space.sm)
                        .border_b(theme().border)
                        .border_color(theme().colors.border)
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .gap(theme().space.sm)
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Worker notices"),
                                )
                                .child(
                                    div()
                                        .text_size(theme().type_scale.caption)
                                        .text_color(theme().colors.subtle)
                                        .child(format!("{count} active")),
                                ),
                        )
                        .child(icon_button(
                            "close-worker-notices",
                            AppIcon::X,
                            "Close worker notices",
                            ButtonTone::Quiet,
                            move |window, cx| {
                                let _ = close.update(cx, |app, cx| app.close_sheet(window, cx));
                            },
                        )),
                )
                .child(
                    div()
                        .id("worker-notices-list")
                        .max_h(px(520.0))
                        .overflow_y_scroll()
                        .when(notices.is_empty(), |body| {
                            body.child(
                                div()
                                    .p(theme().space.md)
                                    .text_color(theme().colors.subtle)
                                    .child("No active coordination notices."),
                            )
                        })
                        .children(
                            notices
                                .into_iter()
                                .enumerate()
                                .map(|(index, notice)| render_notice(index, notice)),
                        ),
                )
                .child(
                    div()
                        .px(theme().space.md)
                        .py(theme().space.sm)
                        .border_t(theme().border)
                        .border_color(theme().colors.border)
                        .text_size(theme().type_scale.caption)
                        .text_color(theme().colors.subtle)
                        .child("Notices expire after 15 minutes"),
                )
        },
    )
}

fn render_notice(index: usize, notice: NoticeView) -> impl IntoElement {
    div()
        .id(("worker-notice", index))
        .px(theme().space.md)
        .py(theme().space.sm)
        .border_b(theme().border)
        .border_color(theme().colors.surface)
        .flex()
        .flex_col()
        .gap(theme().space.xs)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(theme().space.sm)
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(theme().type_scale.body_small)
                        .child(notice.from),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(theme().type_scale.caption)
                        .text_color(theme().colors.subtle)
                        .child(format_age(notice.age_seconds)),
                ),
        )
        .child(
            div()
                .text_size(theme().type_scale.body_small)
                .child(notice.message),
        )
        .when(!notice.paths.is_empty(), |row| {
            row.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(5.0))
                    .children(notice.paths.into_iter().map(|path| {
                        div()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(theme().radius)
                            .bg(theme().colors.surface)
                            .text_size(theme().type_scale.caption)
                            .text_color(theme().colors.muted)
                            .child(path)
                    })),
            )
        })
}

fn format_age(seconds: u64) -> String {
    if seconds < 60 {
        "just now".into()
    } else {
        format!("{}m", seconds / 60)
    }
}
