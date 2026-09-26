use gpui::{
    AnyElement, Context, FocusHandle, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _,
};

use super::{
    super::{FarcasterApp, OVERLAY_KEY_CONTEXT, dialogs},
    keybindings,
};
use crate::app::ui::{
    assets::AppIcon,
    primitives::{ButtonTone, FeedbackTone, feedback, icon_button, modal},
    theme::theme,
};

impl FarcasterApp {
    pub(super) fn render_root_overlays(
        &self,
        root: gpui::Div,
        entity: WeakEntity<Self>,
        picker: Option<AnyElement>,
        work_active: bool,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let task_notice = self.render_code_task_notice(entity.clone());
        let has_notices = !self.extensions.active.notifications.is_empty() || task_notice.is_some();
        let workgraph_focus = self.views.workgraph.read(cx).focus_handle();
        let sessions_sheet = self.overlays.view.sessions.then(|| {
            panel_sheet(
                "sessions",
                "Sessions",
                &self.overlays.sheet_focus,
                entity.clone(),
                self.views
                    .session_rail
                    .clone()
                    .cached(gpui::StyleRefinement::default().size_full())
                    .into_any_element(),
            )
        });
        let run_sheet = self.overlays.view.run.then(|| {
            let reviewing = self.visible_review().is_some();
            let inspecting = self.views.workgraph_inspector_issue.is_some() && !reviewing;
            let content = if inspecting {
                self.views.workgraph_detail.clone().into_any_element()
            } else {
                self.views
                    .run_panel
                    .clone()
                    .cached(gpui::StyleRefinement::default().size_full())
                    .into_any_element()
            };
            panel_sheet(
                "run",
                if reviewing {
                    "Review"
                } else if inspecting {
                    "Node details"
                } else {
                    "Session details"
                },
                &self.overlays.sheet_focus,
                entity.clone(),
                content,
            )
        });

        root.when_some(picker, |root, picker| root.child(picker))
            .when(work_active, |root| {
                let close = entity.clone();
                root.child(modal(
                    "project-work",
                    "Project plan",
                    &workgraph_focus,
                    crate::app::views::workgraph::WORKGRAPH_KEY_CONTEXT,
                    move |window, cx| {
                        let _ = close.update(cx, |this, cx| {
                            this.show_chat_surface(window, cx);
                        });
                    },
                    |surface| {
                        let close = entity.clone();
                        surface
                            .relative()
                            .w(gpui::px(crate::app::views::workgraph::BOARD_WIDTH))
                            .max_w_full()
                            .h(theme().size(620.0))
                            .max_h(gpui::relative(1.0))
                            .overflow_hidden()
                            .child(self.views.workgraph.clone())
                            .child(
                                div()
                                    .absolute()
                                    .top(theme().size(12.0))
                                    .right(theme().space.md)
                                    .child(icon_button(
                                        "close-project-work",
                                        AppIcon::X,
                                        "Close project plan",
                                        ButtonTone::Quiet,
                                        move |window, cx| {
                                            let _ = close.update(cx, |this, cx| {
                                                this.show_chat_surface(window, cx);
                                            });
                                        },
                                    )),
                            )
                    },
                ))
            })
            .when(self.overlays.view.project_trust, |root| {
                root.child(dialogs::project_trust::render(self, entity.clone()))
            })
            .when(self.overlays.view.settings, |root| {
                root.child(dialogs::settings::render(self, entity.clone(), cx))
            })
            .when(self.overlays.view.keybindings, |root| {
                let close = entity.clone();
                root.child(modal(
                    "keybindings-help",
                    "Keyboard shortcuts",
                    &self.overlays.sheet_focus,
                    OVERLAY_KEY_CONTEXT,
                    move |window, cx| {
                        let _ = close.update(cx, |this, cx| this.close_sheet(window, cx));
                    },
                    |surface| {
                        surface
                            .w(theme().size(520.0))
                            .max_w_full()
                            .child(keybindings::render_help())
                    },
                ))
            })
            .when(self.overlays.view.worker_notices, |root| {
                root.child(dialogs::worker_notices::render(self, entity.clone()))
            })
            .when_some(sessions_sheet, |root, sheet| root.child(sheet))
            .when_some(run_sheet, |root, sheet| root.child(sheet))
            .when(self.sessions.pending_archive.is_some(), |root| {
                root.child(dialogs::archive_confirmation::render(self, entity.clone()))
            })
            .when(
                self.workspace.send_to_chat.is_some() && !self.overlays.view.project_trust,
                |root| root.child(dialogs::send_to_chat::render(self, entity.clone(), cx)),
            )
            .when(self.sessions.pending_delete.is_some(), |root| {
                root.child(dialogs::delete_confirmation::render(self, entity.clone()))
            })
            .when(self.sessions.pending_move.is_some(), |root| {
                root.child(dialogs::move_confirmation::render(self, entity.clone()))
            })
            .when(self.sessions.import.is_some(), |root| {
                root.child(dialogs::session_import::render(self, entity.clone()))
            })
            .when(self.project.repository.pending_jj_init.is_some(), |root| {
                root.child(dialogs::jj_init_confirmation::render(self, entity.clone()))
            })
            .when(self.project.repository.edits.pending.is_some(), |root| {
                root.child(dialogs::repository_edit::render(self, entity.clone(), cx))
            })
            .when(self.navigation.pending_model_access.is_some(), |root| {
                root.child(dialogs::model_access_confirmation::render(
                    self,
                    entity.clone(),
                ))
            })
            .when_some(
                dialogs::image_preview::render(self, entity.clone()),
                |root, preview| root.child(preview),
            )
            .when(has_notices, |root| {
                root.child(
                    div()
                        .debug_selector(|| "floating-notices".into())
                        .absolute()
                        .top(theme().space.md)
                        .right(theme().space.md)
                        .w(theme().layout.run_panel)
                        .max_w_full()
                        .flex()
                        .flex_col()
                        .gap(theme().space.xs)
                        .children(task_notice)
                        .children(self.extensions.active.notifications.iter().enumerate().map(
                            |(index, notice)| {
                                feedback(
                                    ("notification", index),
                                    notice.message.clone(),
                                    match notice.tone {
                                        crate::protocol::NotifyTone::Error => FeedbackTone::Error,
                                        crate::protocol::NotifyTone::Warning => {
                                            FeedbackTone::Warning
                                        }
                                        crate::protocol::NotifyTone::Info => FeedbackTone::Info,
                                    },
                                )
                            },
                        )),
                )
            })
            .when(self.lifecycle.pending_quit.is_some(), |root| {
                root.child(dialogs::quit_confirmation::render(self, entity.clone()))
            })
    }
}

#[cfg(test)]
#[path = "overlays_tests.rs"]
mod tests;

fn panel_sheet(
    id: &'static str,
    title: &'static str,
    focus: &FocusHandle,
    entity: WeakEntity<FarcasterApp>,
    content: AnyElement,
) -> AnyElement {
    modal(
        id,
        title,
        focus,
        OVERLAY_KEY_CONTEXT,
        move |window, cx| {
            let _ = entity.update(cx, |this, cx| this.close_sheet(window, cx));
        },
        |surface| surface.h_full().max_w_full().child(content),
    )
    .into_any_element()
}
