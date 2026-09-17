use gpui::{
    IntoElement, ParentElement as _, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _,
};

use crate::app::{
    FarcasterApp,
    ui::{
        assets::AppIcon,
        layout::{LayoutMode, shows_run_sheet_button, shows_session_sheet_button},
        primitives::{AppIconSize, ButtonTone, app_icon, icon_button, icon_control},
        theme::THEME,
    },
};

impl FarcasterApp {
    pub(in crate::app::views) fn render_workspace_panels(
        &self,
        mode: LayoutMode,
        entity: WeakEntity<Self>,
    ) -> impl IntoElement {
        let sessions = entity.clone();
        let work = entity.clone();
        let notice_count = self.worker_notices.snapshot(&self.project.path).len();
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(THEME.space.xs)
            .when(shows_session_sheet_button(mode), |controls| {
                controls.child(icon_button(
                    "open-sessions",
                    AppIcon::ChatCircleDots,
                    "Sessions",
                    ButtonTone::Quiet,
                    move |window, cx| {
                        let _ =
                            sessions.update(cx, |this, cx| this.open_sessions_sheet(window, cx));
                    },
                ))
            })
            .child(icon_button(
                "open-project-work",
                AppIcon::GitFork,
                "Project plan",
                ButtonTone::Quiet,
                move |window, cx| {
                    let _ = work.update(cx, |this, cx| this.open_workgraph_surface(window, cx));
                },
            ))
            .child(worker_notice_control(notice_count, entity.clone()))
            .when(shows_run_sheet_button(mode), |controls| {
                controls.child(icon_button(
                    "open-run",
                    AppIcon::List,
                    "Session details",
                    ButtonTone::Quiet,
                    move |window, cx| {
                        let _ = entity.update(cx, |this, cx| this.open_run_sheet(window, cx));
                    },
                ))
            })
    }
}

fn worker_notice_control(count: usize, entity: WeakEntity<FarcasterApp>) -> impl IntoElement {
    let label = match count {
        1 => "Worker notices — 1 active".to_owned(),
        _ => format!("Worker notices — {count} active"),
    };
    icon_control("open-worker-notices", label)
        .relative()
        .child(app_icon(AppIcon::Chalkboard, AppIconSize::Control))
        .when(count > 0, |control| {
            control.child(
                div()
                    .absolute()
                    .top(gpui::px(1.0))
                    .right(gpui::px(1.0))
                    .min_w(gpui::px(13.0))
                    .h(gpui::px(13.0))
                    .px(gpui::px(3.0))
                    .rounded(gpui::px(7.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(THEME.colors.accent)
                    .text_color(THEME.colors.canvas)
                    .text_size(gpui::px(9.0))
                    .child(if count > 99 {
                        "99+".to_owned()
                    } else {
                        count.to_string()
                    }),
            )
        })
        .on_click(move |_, window, cx| {
            let _ = entity.update(cx, |app, cx| app.open_worker_notices(window, cx));
        })
}
