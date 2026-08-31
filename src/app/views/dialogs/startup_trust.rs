//! Native startup gate shown before a project's first Pi RPC process starts.

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Render, Role, StatefulInteractiveElement as _, Styled as _, WeakEntity,
    Window, div, prelude::FluentBuilder as _, px,
};

use crate::{
    app::FarcasterApp,
    app::ui::primitives::{ButtonTone, button},
    app::ui::theme::{MONO_FONT_FAMILY, THEME},
    projects::{self, StartupTrust, TrustChoice},
};

pub(crate) struct ProjectTrustView {
    project: PathBuf,
    app: Option<Entity<FarcasterApp>>,
    notification_app: Rc<RefCell<Option<WeakEntity<FarcasterApp>>>>,
    approval_ui: crate::access::approval::ApprovalUi,
    workgraph_updates: async_channel::Receiver<()>,
    focus: FocusHandle,
    error: Option<String>,
}

impl ProjectTrustView {
    pub(crate) fn new(
        project: PathBuf,
        startup_trust: StartupTrust,
        notification_app: Rc<RefCell<Option<WeakEntity<FarcasterApp>>>>,
        approval_ui: crate::access::approval::ApprovalUi,
        workgraph_updates: async_channel::Receiver<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let mut this = Self {
            project,
            app: None,
            notification_app,
            approval_ui,
            workgraph_updates,
            focus,
            error: None,
        };
        if startup_trust == StartupTrust::Ready {
            this.start_app(None, window, cx);
        } else {
            let focus = this.focus.clone();
            cx.defer_in(window, move |_, window, cx| focus.focus(window, cx));
        }
        this
    }

    fn select_trust(&mut self, choice: TrustChoice, window: &mut Window, cx: &mut Context<Self>) {
        match projects::apply(&self.project, choice) {
            Ok(applied) => self.start_app(Some(applied.trusted), window, cx),
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    fn start_app(
        &mut self,
        repository_execution_allowed: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self.project.clone();
        let repository_execution_allowed = repository_execution_allowed
            .unwrap_or_else(|| projects::repository_execution_allowed(&project).unwrap_or(false));
        let approval_ui = self.approval_ui.clone();
        let workgraph_updates = self.workgraph_updates.clone();
        let app = cx.new(|cx| {
            FarcasterApp::new(
                project,
                repository_execution_allowed,
                approval_ui,
                workgraph_updates,
                window,
                cx,
            )
        });
        *self.notification_app.borrow_mut() = Some(app.downgrade());
        self.app = Some(app);
        cx.notify();
    }
}

impl Render for ProjectTrustView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        if let Some(app) = &self.app {
            return app.clone().into_any_element();
        }

        let entity = cx.entity().downgrade();
        let project = self.project.display().to_string();
        let mut options = div().flex().flex_col().gap(THEME.space.xs);
        for (index, option) in projects::options(&self.project).into_iter().enumerate() {
            let choice = option.choice;
            let select = entity.clone();
            let tone = if index == 0 {
                ButtonTone::Accent
            } else {
                ButtonTone::Neutral
            };
            options = options.child(
                button(
                    ("startup-trust-option", index),
                    option.label,
                    tone,
                    true,
                    move |window, cx| {
                        let _ = select.update(cx, |this, cx| {
                            this.select_trust(choice, window, cx);
                        });
                    },
                )
                .w_full(),
            );
        }

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(THEME.colors.canvas)
            .p(THEME.space.md)
            .child(
                div()
                    .id("startup-project-trust")
                    .role(Role::Group)
                    .aria_label("Project trust")
                    .track_focus(&self.focus)
                    .key_context("PiProjectTrust")
                    .w_full()
                    .max_w(px(640.0))
                    .flex()
                    .flex_col()
                    .gap(THEME.space.md)
                    .rounded(THEME.radius)
                    .border(THEME.border)
                    .border_color(THEME.colors.border)
                    .bg(THEME.colors.panel)
                    .p(THEME.space.md)
                    .child(
                        div()
                            .text_size(THEME.type_scale.display)
                            .text_color(THEME.colors.text)
                            .child("Trust project folder?"),
                    )
                    .child(
                        div()
                            .font_family(MONO_FONT_FAMILY)
                            .text_size(THEME.type_scale.body_small)
                            .text_color(THEME.colors.accent)
                            .child(project),
                    )
                    .child(
                        div()
                            .line_height(THEME.type_scale.line_body)
                            .text_color(THEME.colors.muted)
                            .child("Trusting allows Pi to load project settings and resources, install missing project packages, and execute project extensions."),
                    )
                    .when_some(self.error.clone(), |panel, error| {
                        panel.child(
                            div()
                                .text_color(THEME.colors.error)
                                .child(format!("Trust decision was not saved: {error}")),
                        )
                    })
                    .child(options),
            )
            .into_any_element()
    }
}
