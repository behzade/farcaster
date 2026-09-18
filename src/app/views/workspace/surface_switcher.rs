use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::tooltip::Tooltip;

use crate::{
    app::ui::assets::AppIcon,
    app::ui::primitives::{AppIconSize, app_icon, icon_control},
    app::ui::theme::theme,
    app::workspace::editor::effective_editor_choice,
    app::{AppSurface, FarcasterApp, views::session_rail::project_label},
};

impl FarcasterApp {
    pub(in crate::app::views) fn render_workspace_bar(
        &self,
        entity: WeakEntity<Self>,
        mode: crate::app::ui::layout::LayoutMode,
    ) -> impl IntoElement {
        let project_path = self.workspace_project();
        let project = project_label(&project_path);
        let project_hint = format!("New session in {project}");
        let project_entity = entity.clone();
        let selected_path = self.snapshot.selected_session.as_deref();
        let session = selected_path.and_then(|path| {
            self.sessions
                .all
                .iter()
                .find(|session| session.path == path)
        });
        let harness_icon = session
            .map(|session| AppIcon::for_harness(session.harness))
            .or_else(|| {
                let selected = self.sessions.selected_draft.as_deref()?;
                self.sessions
                    .drafts
                    .iter()
                    .find(|draft| draft.id == selected)
                    .map(|draft| AppIcon::for_harness(draft.harness))
            })
            .unwrap_or(AppIcon::Pi);
        let title = session.map(|session| session.title.clone()).or_else(|| {
            let selected = self.sessions.selected_draft.as_deref()?;
            self.sessions
                .drafts
                .iter()
                .find(|draft| draft.id == selected)
                .and_then(|draft| draft.title.clone())
        });

        div()
            .h(theme().size(38.0))
            .flex_none()
            .flex()
            .items_center()
            .gap(theme().space.sm)
            .px(theme().size(12.0))
            .border_b(theme().border)
            .border_color(theme().colors.surface)
            .bg(theme().colors.canvas)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap(theme().space.sm)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(theme().type_scale.caption)
                    .child(
                        app_icon(AppIcon::Folder, AppIconSize::Inline)
                            .text_color(theme().colors.subtle),
                    )
                    .child(
                        div()
                            .id("workspace-project-new-session")
                            .role(gpui::Role::Button)
                            .aria_label(project_hint.clone())
                            .tab_index(0)
                            .cursor_pointer()
                            .text_color(theme().colors.muted)
                            .hover(|link| link.text_color(theme().colors.text))
                            .focus_visible(|link| link.text_color(theme().colors.indicator))
                            .tooltip(move |window, cx| {
                                Tooltip::new(project_hint.clone()).build(window, cx)
                            })
                            .child(project)
                            .on_click(move |_, window, cx| {
                                let _ = project_entity.update(cx, |app, cx| {
                                    app.new_session(project_path.clone(), window, cx);
                                });
                            }),
                    )
                    .when_some(title, |workspace, title| {
                        workspace
                            .child(div().text_color(theme().colors.subtle).child("/"))
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(theme().colors.text)
                                    .child(title),
                            )
                    }),
            )
            .child(self.render_workspace_panels(mode, entity.clone()))
            .child(
                div()
                    .w(theme().size(1.0))
                    .h(theme().space.md)
                    .bg(theme().colors.surface),
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
        let editor =
            effective_editor_choice(self.settings.editor_choice, &self.workspace_project());
        div()
            .h_full()
            .flex()
            .items_center()
            .gap(theme().size(2.0))
            .child(surface_control(
                "show-chat-surface",
                chat_hint,
                harness_icon,
                self.workspace.surface == AppSurface::Chat,
                entity.clone(),
                FarcasterApp::show_chat_surface,
            ))
            .child(surface_control(
                "show-editor-surface",
                format!(
                    "{} ({modifier}+E in app views; {} anywhere)",
                    editor.label(),
                    crate::app::ui::navigation::command_key(
                        crate::app::ui::navigation::Command::Editor
                    )
                ),
                AppIcon::for_editor(editor),
                self.workspace.surface == AppSurface::Editor,
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
                self.workspace.surface == AppSurface::Terminal,
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
        .w(theme().size(34.0))
        .h_full()
        .rounded_none()
        .hover(|control| control.bg(theme().colors.surface))
        .when(active, |control| {
            control
                .border_b(theme().size(2.0))
                .border_color(theme().colors.indicator)
                .text_color(theme().colors.indicator)
        })
        .child(app_icon(icon, AppIconSize::Control))
        .on_click(move |_, window, cx| {
            let _ = entity.update(cx, |app, cx| action(app, window, cx));
        })
}
