use gpui::{
    IntoElement, ParentElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _,
};

use crate::app::{
    FarcasterApp,
    ui::{
        assets::AppIcon,
        layout::{LayoutMode, shows_run_sheet_button, shows_session_sheet_button},
        primitives::{ButtonTone, icon_button},
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
