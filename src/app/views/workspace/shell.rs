use gpui::{
    IntoElement, ParentElement as _, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _,
};

use crate::app::{
    FarcasterApp,
    ui::{
        assets::AppIcon,
        layout::{
            LayoutMode, shows_right_inline, shows_run_sheet_button, shows_session_sheet_button,
        },
        primitives::{AppIconSize, ButtonTone, app_icon, icon_button, icon_control, number_slot},
        theme::theme,
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
        let panel_toggle = entity.clone();
        let notice_count = self.worker_notices.snapshot(&self.project.path).len();
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(theme().space.xs)
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
            .when(shows_right_inline(mode), |controls| {
                controls.child(icon_button(
                    "toggle-run-panel",
                    AppIcon::SidebarLeft,
                    if self.workspace.run_panel_hidden {
                        "Show source control"
                    } else {
                        "Hide source control"
                    },
                    ButtonTone::Quiet,
                    move |_, cx| {
                        let _ = panel_toggle.update(cx, |this, cx| this.toggle_run_panel(cx));
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
                    .top(theme().size(1.0))
                    .right(theme().size(1.0))
                    .child(number_slot(
                        if count > 99 {
                            "99+".to_owned()
                        } else {
                            count.to_string()
                        },
                        theme().layout.counter_slot,
                    )),
            )
        })
        .on_click(move |_, window, cx| {
            let _ = entity.update(cx, |app, cx| app.open_worker_notices(window, cx));
        })
}
