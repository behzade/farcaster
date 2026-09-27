use gpui::{
    InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, WeakEntity, div, prelude::FluentBuilder as _,
};
use gpui_component::IconName;

use crate::app::{
    FarcasterApp,
    ui::{
        assets::AppIcon,
        layout::{
            LayoutMode, shows_right_inline, shows_run_sheet_button, shows_session_sheet_button,
        },
        primitives::{AppIconSize, ButtonTone, app_icon, icon_button, icon_control},
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
            .when(
                shows_right_inline(mode) && self.workspace.run_panel_hidden,
                |controls| {
                    controls.child(icon_button(
                        "toggle-run-panel",
                        AppIcon::SidebarLeft,
                        "Show source control",
                        ButtonTone::Quiet,
                        move |_, cx| {
                            let _ = panel_toggle.update(cx, |this, cx| this.toggle_run_panel(cx));
                        },
                    ))
                },
            )
    }
}

fn worker_notice_control(count: usize, entity: WeakEntity<FarcasterApp>) -> impl IntoElement {
    let label = format!("Worker notices — {count} active");
    icon_control("open-worker-notices", label)
        .relative()
        .text_color(theme().colors.muted)
        .hover(|control| {
            control
                .bg(theme().colors.highlight)
                .text_color(theme().colors.text)
        })
        .active(|control| {
            control
                .bg(theme().colors.highlight)
                .text_color(theme().colors.text)
        })
        .focus_visible(|control| {
            control
                .border(theme().border)
                .border_color(theme().colors.accent)
                .text_color(theme().colors.text)
        })
        .child(app_icon(IconName::Inbox, AppIconSize::Control))
        .when(count > 0, |control| {
            control.child(
                div()
                    .absolute()
                    .top(theme().size(2.0))
                    .right(theme().size(2.0))
                    .size(theme().size(6.0))
                    .rounded_full()
                    .bg(theme().colors.indicator)
                    .border(theme().border)
                    .border_color(theme().colors.canvas),
            )
        })
        .on_click(move |_, window, cx| {
            let _ = entity.update(cx, |app, cx| app.open_worker_notices(window, cx));
        })
}
