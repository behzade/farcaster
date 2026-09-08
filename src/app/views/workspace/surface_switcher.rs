use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div,
    prelude::FluentBuilder as _,
};

use crate::{
    app::ui::assets::AppIcon,
    app::ui::primitives::{AppIconSize, app_icon, icon_control},
    app::ui::theme::THEME,
    app::{AppSurface, FarcasterApp, views::session_rail::project_label},
};

impl FarcasterApp {
    pub(in crate::app::views) fn render_workspace_bar(
        &self,
        entity: WeakEntity<Self>,
        mode: crate::app::ui::layout::LayoutMode,
    ) -> impl IntoElement {
        let project = project_label(&self.workspace_project());
        let selected_path = self.snapshot.selected_session.as_deref();
        let session = selected_path.and_then(|path| {
            self.all_sessions
                .iter()
                .find(|session| session.path == path)
        });
        let harness_icon = session
            .map(|session| AppIcon::for_harness(&session.harness))
            .or_else(|| {
                let selected = self.selected_draft.as_deref()?;
                self.drafts
                    .iter()
                    .find(|draft| draft.id == selected)
                    .map(|draft| AppIcon::for_harness(&draft.harness))
            })
            .unwrap_or(AppIcon::Pi);
        let title = session.map(|session| session.title.clone()).or_else(|| {
            let selected = self.selected_draft.as_deref()?;
            self.drafts
                .iter()
                .find(|draft| draft.id == selected)
                .and_then(|draft| draft.title.clone())
        });

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
                    .flex()
                    .items_center()
                    .gap(THEME.space.sm)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(THEME.type_scale.caption)
                    .child(
                        app_icon(AppIcon::Folder, AppIconSize::Inline)
                            .text_color(THEME.colors.subtle),
                    )
                    .child(div().text_color(THEME.colors.muted).child(project))
                    .when_some(title, |workspace, title| {
                        workspace
                            .child(div().text_color(THEME.colors.subtle).child("/"))
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(THEME.colors.text)
                                    .child(title),
                            )
                    }),
            )
            .child(self.render_workspace_panels(mode, entity.clone()))
            .child(
                div()
                    .w(gpui::px(1.0))
                    .h(THEME.space.md)
                    .bg(THEME.colors.surface),
            )
            .child(self.render_surface_switcher(entity, harness_icon))
    }

    pub(in crate::app::views) fn render_surface_switcher(
        &self,
        entity: WeakEntity<Self>,
        harness_icon: AppIcon,
    ) -> impl IntoElement {
        let (modifier, chat_hint) = if cfg!(target_os = "macos") {
            (
                "Cmd",
                "Chat composer (Cmd+G in app views; Ctrl+G Ctrl+G anywhere)",
            )
        } else {
            ("Ctrl", "Chat composer (Ctrl+G Ctrl+G anywhere)")
        };
        div()
            .h_full()
            .flex()
            .items_center()
            .gap(gpui::px(2.0))
            .child(surface_control(
                "show-chat-surface",
                chat_hint,
                harness_icon,
                self.surface == AppSurface::Chat,
                entity.clone(),
                FarcasterApp::show_chat_surface,
            ))
            .child(surface_control(
                "show-editor-surface",
                format!(
                    "Neovim ({modifier}+E in app views; {} anywhere)",
                    crate::app::ui::navigation::command_key(
                        crate::app::ui::navigation::Command::Editor
                    )
                ),
                AppIcon::Neovim,
                self.surface == AppSurface::Editor,
                entity.clone(),
                FarcasterApp::show_editor_surface,
            ))
            .child(surface_control(
                "show-terminal-surface",
                format!(
                    "Terminal ({modifier}+T in app views; {} anywhere)",
                    crate::app::ui::navigation::command_key(
                        crate::app::ui::navigation::Command::Terminal
                    )
                ),
                AppIcon::Ghostty,
                self.surface == AppSurface::Terminal,
                entity,
                FarcasterApp::show_terminal_surface,
            ))
    }
}

type SurfaceAction = fn(&mut FarcasterApp, &mut Window, &mut Context<FarcasterApp>);

fn surface_control(
    id: &'static str,
    label: impl Into<gpui::SharedString>,
    icon: AppIcon,
    active: bool,
    entity: WeakEntity<FarcasterApp>,
    action: SurfaceAction,
) -> gpui::Stateful<gpui::Div> {
    icon_control(id, label)
        .w(gpui::px(34.0))
        .h_full()
        .rounded_none()
        .hover(|control| control.bg(THEME.colors.surface))
        .when(active, |control| {
            control
                .border_b(gpui::px(2.0))
                .border_color(THEME.colors.accent)
                .text_color(THEME.colors.accent)
        })
        .child(app_icon(icon, AppIconSize::Control))
        .on_click(move |_, window, cx| {
            let _ = entity.update(cx, |app, cx| action(app, window, cx));
        })
}
