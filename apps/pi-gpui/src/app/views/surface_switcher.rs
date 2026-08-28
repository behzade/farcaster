//! Visible controls for the center workspace surfaces and input-routing mode.

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div,
    prelude::FluentBuilder as _,
};

use super::super::{AppSurface, PiApp};
use crate::{
    assets::AppIcon,
    primitives::{AppIconSize, app_icon, icon_control},
    theme::THEME,
};

const fn input_mode_label(surface: AppSurface) -> &'static str {
    match surface {
        AppSurface::Chat | AppSurface::Work => "NORMAL",
        AppSurface::Editor | AppSurface::Terminal => "INSERT",
    }
}

impl PiApp {
    pub(super) fn render_surface_switcher(&self, entity: WeakEntity<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(THEME.space.xs)
            .child(surface_control(
                "show-chat-surface",
                "Chat (F1)",
                AppIcon::ChatCircleDots,
                self.surface == AppSurface::Chat,
                entity.clone(),
                PiApp::show_chat_surface,
            ))
            .child(surface_control(
                "show-editor-surface",
                "Neovim (F2)",
                AppIcon::Code,
                self.surface == AppSurface::Editor,
                entity.clone(),
                PiApp::show_editor_surface,
            ))
            .child(surface_control(
                "show-terminal-surface",
                "Terminal (F3)",
                AppIcon::TerminalWindow,
                self.surface == AppSurface::Terminal,
                entity,
                PiApp::show_terminal_surface,
            ))
            .child(
                div()
                    .h(THEME.controls.icon_button)
                    .px(THEME.space.xs)
                    .flex()
                    .items_center()
                    .rounded(THEME.radius)
                    .bg(THEME.colors.surface)
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.muted)
                    .child(input_mode_label(self.surface)),
            )
    }

    pub(super) fn render_floating_surface_switcher(
        &self,
        entity: WeakEntity<Self>,
    ) -> impl IntoElement {
        let hover = entity.clone();
        div()
            .id("floating-surface-switcher")
            .absolute()
            .top(THEME.space.sm)
            .left(THEME.space.sm)
            .h(gpui::px(36.0))
            .flex()
            .items_center()
            .p(THEME.space.xs)
            .rounded(THEME.radius)
            .bg(THEME.colors.panel)
            .border(THEME.border)
            .border_color(THEME.colors.border)
            .when(
                matches!(self.surface, AppSurface::Editor | AppSurface::Terminal),
                |switcher| {
                    switcher.on_hover(move |hovered, window, cx| {
                        let _ = hover.update(cx, |app, cx| {
                            if *hovered {
                                app.hide_native_workspace_surfaces(cx);
                            } else {
                                app.restore_active_native_workspace_surface(window, cx);
                            }
                        });
                    })
                },
            )
            .child(self.render_surface_switcher(entity))
    }
}

type SurfaceAction = fn(&mut PiApp, &mut Window, &mut Context<PiApp>);

fn surface_control(
    id: &'static str,
    label: &'static str,
    icon: AppIcon,
    active: bool,
    entity: WeakEntity<PiApp>,
    action: SurfaceAction,
) -> gpui::Stateful<gpui::Div> {
    icon_control(id, label)
        .hover(|control| control.bg(THEME.colors.hover))
        .when(active, |control| {
            control
                .bg(THEME.colors.surface)
                .text_color(THEME.colors.accent)
        })
        .child(app_icon(icon, AppIconSize::Control))
        .on_click(move |_, window, cx| {
            let _ = entity.update(cx, |app, cx| action(app, window, cx));
        })
}
