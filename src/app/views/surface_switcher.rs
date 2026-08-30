use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div,
    prelude::FluentBuilder as _,
};

use super::{
    super::{AppSurface, FarcasterApp},
    session_rows::project_label,
};
use crate::{
    assets::AppIcon,
    primitives::{AppIconSize, app_icon, icon_control},
    sessions::root_session_for_path,
    theme::THEME,
};

impl FarcasterApp {
    pub(super) fn render_workspace_bar(&self, entity: WeakEntity<Self>) -> impl IntoElement {
        let project = project_label(&self.workspace_project());
        let title =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .map(|session| session.title.as_str())
                .or_else(|| {
                    let selected = self.selected_draft.as_deref()?;
                    self.drafts
                        .iter()
                        .find(|draft| draft.id == selected)
                        .and_then(|draft| draft.title.as_deref())
                });
        let workspace =
            title.map_or_else(|| project.clone(), |title| format!("{project} / {title}"));

        div()
            .h(gpui::px(38.0))
            .flex_none()
            .flex()
            .items_center()
            .gap(THEME.space.sm)
            .px(gpui::px(12.0))
            .border_b(THEME.border)
            .border_color(THEME.colors.surface)
            .bg(THEME.colors.canvas)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.muted)
                    .child(workspace),
            )
            .child(self.render_surface_switcher(entity))
    }

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
                FarcasterApp::show_chat_surface,
            ))
            .child(surface_control(
                "show-editor-surface",
                "Neovim (F2)",
                AppIcon::Code,
                self.surface == AppSurface::Editor,
                entity.clone(),
                FarcasterApp::show_editor_surface,
            ))
            .child(surface_control(
                "show-terminal-surface",
                "Terminal (F3)",
                AppIcon::TerminalWindow,
                self.surface == AppSurface::Terminal,
                entity,
                FarcasterApp::show_terminal_surface,
            ))
    }
}

type SurfaceAction = fn(&mut FarcasterApp, &mut Window, &mut Context<FarcasterApp>);

fn surface_control(
    id: &'static str,
    label: &'static str,
    icon: AppIcon,
    active: bool,
    entity: WeakEntity<FarcasterApp>,
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
