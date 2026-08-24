//! Hover-revealed controls for the center workspace surfaces.

use gpui::{
    Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div,
    prelude::FluentBuilder as _,
};

use super::super::{AppSurface, PiApp};
use crate::{
    assets::AppIcon,
    primitives::{AppIconSize, app_icon, icon_control},
    theme::THEME,
};

const SURFACE_GROUP: &str = "center-surface-switcher";

impl PiApp {
    pub(super) fn render_surface_switcher(
        &self,
        entity: WeakEntity<Self>,
        overlaps_native_surface: bool,
    ) -> impl IntoElement {
        let hover = entity.clone();
        div()
            .absolute()
            .top(THEME.space.sm)
            .left(THEME.space.sm)
            .w(gpui::px(12.0))
            .h(gpui::px(36.0))
            .flex()
            .items_center()
            .gap(THEME.space.xs)
            .p(THEME.space.xs)
            .rounded(THEME.radius)
            .overflow_hidden()
            .group(SURFACE_GROUP)
            .hover(|switcher| {
                switcher
                    .w(gpui::px(108.0))
                    .bg(THEME.colors.panel)
                    .border(THEME.border)
                    .border_color(THEME.colors.border)
            })
            .when(overlaps_native_surface, |switcher| {
                switcher.on_hover(move |hovered, _, cx| {
                    let _ = hover.update(cx, |app, cx| {
                        if *hovered {
                            app.hide_native_workspace_surfaces(cx);
                        } else {
                            app.restore_active_native_workspace_surface(cx);
                        }
                    });
                })
            })
            .child(surface_control(
                "show-chat-surface",
                "Chat (⌘L)",
                AppIcon::ChatCircleDots,
                self.surface == AppSurface::Chat,
                entity.clone(),
                PiApp::show_chat_surface,
            ))
            .child(surface_control(
                "show-editor-surface",
                "Neovim (⌘E)",
                AppIcon::Code,
                self.surface == AppSurface::Editor,
                entity.clone(),
                PiApp::show_editor_surface,
            ))
            .child(surface_control(
                "show-terminal-surface",
                "Terminal (⌘T)",
                AppIcon::TerminalWindow,
                self.surface == AppSurface::Terminal,
                entity,
                PiApp::show_terminal_surface,
            ))
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
        .opacity(0.0)
        .group_hover(SURFACE_GROUP, |control| control.opacity(1.0))
        .focus(|control| control.opacity(1.0))
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
