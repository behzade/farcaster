use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div,
    prelude::FluentBuilder as _,
};

use crate::{
    app::ui::assets::AppIcon,
    app::ui::primitives::{AppIconSize, ButtonTone, app_icon, button, icon_control},
    app::ui::theme::THEME,
    app::{AppSurface, FarcasterApp, views::session_rail::project_label},
    sessions::root_session_for_path,
};

impl FarcasterApp {
    pub(in crate::app::views) fn render_workspace_bar(
        &self,
        entity: WeakEntity<Self>,
        mode: crate::app::ui::layout::LayoutMode,
    ) -> impl IntoElement {
        let project = project_label(&self.workspace_project());
        let selected_path = self.snapshot.selected_session.as_deref();
        let session = root_session_for_path(&self.sessions, selected_path);
        let harness_icon = selected_path
            .and_then(|path| self.sessions.iter().find(|session| session.path == path))
            .or(session)
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
            .when(
                self.surface == AppSurface::Chat && self.selected_draft_is_empty_and_unsubmitted(),
                |bar| {
                    let sessions = entity.clone();
                    let work = entity.clone();
                    let details = entity.clone();
                    bar.when(
                        crate::app::ui::layout::shows_session_sheet_button(mode),
                        |bar| {
                            bar.child(button(
                                "draft-sessions",
                                "Sessions",
                                ButtonTone::Quiet,
                                true,
                                move |window, cx| {
                                    let _ = sessions.update(cx, |this, cx| {
                                        this.open_sessions_sheet(window, cx)
                                    });
                                },
                            ))
                        },
                    )
                    .when(crate::app::ui::layout::shows_draft_inspector(mode, self.overlays.draft_inspector), |bar| {
                        bar.child(button(
                            "hide-draft-details", "Hide session details", ButtonTone::Quiet, true,
                            move |_, cx| {
                                let _ = details.update(cx, |this, cx| {
                                    this.overlays.draft_inspector = false;
                                    if let Err(error) = crate::app::infrastructure::persistence::StateStore::open()
                                        .and_then(|store| store.save_draft_inspector(false))
                                    {
                                        this.sessions_error = Some(error);
                                    }
                                    cx.notify();
                                });
                            },
                        ))
                    })
                    .child(button(
                        "draft-project-work",
                        "Project work",
                        ButtonTone::Quiet,
                        true,
                        move |window, cx| {
                            let _ =
                                work.update(cx, |this, cx| this.open_workgraph_surface(window, cx));
                        },
                    ))
                },
            )
            .child(self.render_surface_switcher(entity, harness_icon))
    }

    pub(in crate::app::views) fn render_surface_switcher(
        &self,
        entity: WeakEntity<Self>,
        harness_icon: AppIcon,
    ) -> impl IntoElement {
        div()
            .h_full()
            .flex()
            .items_center()
            .gap(gpui::px(2.0))
            .child(surface_control(
                "show-chat-surface",
                format!(
                    "Chat ({})",
                    crate::app::ui::keybindings::application_key("l")
                ),
                harness_icon,
                self.surface == AppSurface::Chat,
                entity.clone(),
                FarcasterApp::show_chat_surface,
            ))
            .child(surface_control(
                "show-editor-surface",
                format!(
                    "Neovim ({})",
                    crate::app::ui::keybindings::application_key("e")
                ),
                AppIcon::Neovim,
                self.surface == AppSurface::Editor,
                entity.clone(),
                FarcasterApp::show_editor_surface,
            ))
            .child(surface_control(
                "show-terminal-surface",
                format!(
                    "Terminal ({})",
                    crate::app::ui::keybindings::application_key("j")
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
