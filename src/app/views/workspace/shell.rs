use gpui::{
    IntoElement, ParentElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _,
};

use crate::app::{
    FarcasterApp,
    ui::{
        assets::AppIcon,
        layout::{
            LayoutMode, shows_right_inline, shows_run_sheet_button, shows_session_sheet_button,
        },
        primitives::{ButtonTone, icon_button},
    },
};

impl FarcasterApp {
    pub(in crate::app::views) fn render_workspace_panels(
        &self,
        mode: LayoutMode,
        entity: WeakEntity<Self>,
    ) -> impl IntoElement {
        let sessions = entity.clone();
        let panel_toggle = entity.clone();
        div()
            .flex_none()
            .h_full()
            .flex()
            .items_center()
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
