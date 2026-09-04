use gpui::{
    AnyElement, Context, FocusHandle, IntoElement as _, ParentElement as _, Styled as _,
    WeakEntity, div, prelude::FluentBuilder as _,
};

use super::{
    super::{FarcasterApp, OVERLAY_KEY_CONTEXT, dialogs},
    keybindings,
};
use crate::app::ui::{
    assets::AppIcon,
    primitives::{ButtonTone, FeedbackTone, feedback, icon_button, modal},
    theme::THEME,
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
        let workgraph_focus = self.workgraph_view.read(cx).focus_handle();
        let sessions_sheet = self.view.overlays.sessions.then(|| {
            panel_sheet(
                "sessions",
                "Sessions",
                &self.sheet_focus,
                entity.clone(),
                self.session_rail_view
                    .clone()
                    .cached(gpui::StyleRefinement::default().size_full())
                    .into_any_element(),
            )
        });
        let run_sheet = self.view.overlays.run.then(|| {
            let inspecting = self.workgraph_inspector_issue.is_some();
            let content = if inspecting {
                self.workgraph_detail_view.clone().into_any_element()
            } else {
                self.run_panel_view
                    .clone()
                    .cached(gpui::StyleRefinement::default().size_full())
                    .into_any_element()
            };
            panel_sheet(
                "run",
                if inspecting {
                    "Node details"
                } else {
                    "Session details"
                },
                &self.sheet_focus,
                entity.clone(),
                content,
            )
        });

        root.when_some(picker, |root, picker| root.child(picker))
            .when(work_active, |root| {
                let close = entity.clone();
                root.child(modal(
                    "project-work",
                    "Plans",
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
                            .w(gpui::px(820.0))
                            .max_w_full()
                            .h(gpui::px(620.0))
                            .max_h(gpui::relative(1.0))
                            .overflow_hidden()
                            .child(
                                div()
                                    .size_full()
                                    .min_h_0()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .h(gpui::px(48.0))
                                            .flex_none()
                                            .px(THEME.space.md)
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .border_b(THEME.border)
                                            .border_color(THEME.colors.border)
                                            .child("Plans")
                                            .child(icon_button(
                                                "close-project-work",
                                                AppIcon::X,
                                                "Close plans",
                                                ButtonTone::Quiet,
                                                move |window, cx| {
                                                    let _ = close.update(cx, |this, cx| {
                                                        this.show_chat_surface(window, cx);
                                                    });
                                                },
                                            )),
                                    )
                                    .child(
                                        div().flex_1().min_h_0().child(self.workgraph_view.clone()),
                                    ),
                            )
                    },
                ))
            })
            .when_some(
                dialogs::image_preview::render(self, entity.clone()),
                |root, preview| root.child(preview),
            )
            .when(self.pending_archive.is_some(), |root| {
                root.child(dialogs::archive_confirmation::render(self, entity.clone()))
            })
            .when(self.pending_delete.is_some(), |root| {
                root.child(dialogs::delete_confirmation::render(self, entity.clone()))
            })
            .when(self.repository.pending_jj_init.is_some(), |root| {
                root.child(dialogs::jj_init_confirmation::render(self, entity.clone()))
            })
            .when(self.view.overlays.project_trust, |root| {
                root.child(dialogs::project_trust::render(self, entity.clone()))
            })
            .when(self.view.overlays.settings, |root| {
                root.child(dialogs::settings::render(self, entity.clone()))
            })
            .when(self.view.overlays.keybindings, |root| {
                let close = entity.clone();
                root.child(modal(
                    "keybindings-help",
                    "Keyboard shortcuts",
                    &self.sheet_focus,
                    OVERLAY_KEY_CONTEXT,
                    move |window, cx| {
                        let _ = close.update(cx, |this, cx| this.close_sheet(window, cx));
                    },
                    |surface| {
                        surface
                            .w(gpui::px(520.0))
                            .max_w_full()
                            .child(keybindings::render_help())
                    },
                ))
            })
            .when_some(sessions_sheet, |root, sheet| root.child(sheet))
            .when_some(run_sheet, |root, sheet| root.child(sheet))
            .when(!self.extension.notifications.is_empty(), |root| {
                root.child(
                    div()
                        .absolute()
                        .top(THEME.space.md)
                        .right(THEME.space.md)
                        .w(THEME.layout.run_panel)
                        .max_w_full()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.xs)
                        .children(self.extension.notifications.iter().enumerate().map(
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
    }
}

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
