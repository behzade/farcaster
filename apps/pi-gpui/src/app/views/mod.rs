mod composer;
mod shell;

use gpui::{
    Context, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::FocusTrapElement as _;

use super::{DismissSurface, PiApp};
pub(crate) const OVERLAY_KEY_CONTEXT: &str = "PiGpuiOverlay";

use crate::{
    layout::{LayoutMode, layout_mode},
    primitives::{FeedbackTone, dialog_backdrop, dialog_surface, feedback},
    protocol::ExtensionUiRequest,
    theme::THEME,
    transcript,
};

impl Render for PiApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.resolve_pending_submission(window, cx);
        if self.pending_session_reset {
            self.pending_session_reset = false;
            let focus = self.composer_focus.clone();
            cx.defer_in(window, move |_, window, cx| focus.focus(window, cx));
        }
        if self.pending_sheet_setup {
            self.pending_sheet_setup = false;
            let focus = self.sheet_focus.clone();
            cx.defer_in(window, move |_, window, cx| focus.focus(window, cx));
        }
        if self.pending_dialog_setup {
            self.pending_dialog_setup = false;
            if self.dialog_return_focus.is_none() {
                self.dialog_return_focus = window.focused(cx);
            }
            let (prefill, uses_input) = match self.extension.dialog.as_ref() {
                Some(ExtensionUiRequest::Editor { prefill, .. }) => {
                    (prefill.clone().unwrap_or_default(), true)
                }
                Some(ExtensionUiRequest::Input { .. }) => (String::new(), true),
                _ => (String::new(), false),
            };
            let input = self.dialog_input.clone();
            let focus = if uses_input {
                input.read(cx).focus_handle(cx)
            } else {
                self.dialog_focus.clone()
            };
            cx.defer_in(window, move |_, window, cx| {
                input.update(cx, |state, cx| state.set_value(prefill, window, cx));
                focus.focus(window, cx);
            });
        }
        if let Some((generation, title)) = self.pending_title.take() {
            cx.defer_in(window, move |this, window, _| {
                if this.runtime_generation == generation {
                    window.set_window_title(&title);
                }
            });
        }
        if let Some((generation, text)) = self.pending_editor_text.take() {
            cx.defer_in(window, move |this, window, cx| {
                if this.runtime_generation == generation {
                    this.composer
                        .update(cx, |state, cx| state.set_value(text, window, cx));
                }
            });
        }
        let mode = layout_mode(window.viewport_size().width);
        let entity = cx.entity().downgrade();
        let main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(self.render_header(mode, entity.clone()))
            .child(div().flex_1().min_h_0().child(transcript::render(
                &self.transcript_list,
                self.transcript_following,
                self.transcript_unseen,
                entity.clone(),
                cx,
            )))
            .child(self.render_composer(entity.clone()));
        div()
            .relative()
            .size_full()
            .bg(THEME.colors.canvas)
            .text_color(THEME.colors.text)
            .text_size(THEME.type_scale.body)
            .on_action(cx.listener(|this, _: &DismissSurface, window, cx| {
                this.dismiss_surface(window, cx);
            }))
            .child(
                div()
                    .size_full()
                    .flex()
                    .when(mode != LayoutMode::Narrow, |shell| {
                        shell.child(
                            div()
                                .w(if mode == LayoutMode::Wide {
                                    THEME.layout.session_rail
                                } else {
                                    THEME.layout.collapsed_rail
                                })
                                .flex_none()
                                .border_r(THEME.border)
                                .border_color(THEME.colors.border)
                                .child(
                                    self.render_sessions(
                                        mode == LayoutMode::Compact,
                                        entity.clone(),
                                    ),
                                ),
                        )
                    })
                    .child(main)
                    .when(mode == LayoutMode::Wide, |shell| {
                        shell.child(
                            div()
                                .w(THEME.layout.run_panel)
                                .flex_none()
                                .border_l(THEME.border)
                                .border_color(THEME.colors.border)
                                .child(self.render_run_panel(entity.clone())),
                        )
                    }),
            )
            .when(self.sessions_sheet, |root| {
                let close = entity.clone();
                root.child(
                    dialog_backdrop("sessions-sheet", move |window, cx| {
                        let _ = close.update(cx, |this, cx| this.close_sheet(window, cx));
                    })
                    .child(
                        dialog_surface("sessions-dialog", "Sessions")
                            .track_focus(&self.sheet_focus)
                            .key_context(OVERLAY_KEY_CONTEXT)
                            .h_full()
                            .max_w_full()
                            .child(self.render_sessions(false, entity.clone()))
                            .focus_trap("sessions-sheet-trap", &self.sheet_focus),
                    ),
                )
            })
            .when(self.run_sheet, |root| {
                let close = entity.clone();
                root.child(
                    dialog_backdrop("run-sheet", move |window, cx| {
                        let _ = close.update(cx, |this, cx| this.close_sheet(window, cx));
                    })
                    .child(
                        dialog_surface("run-dialog", "Run details")
                            .track_focus(&self.sheet_focus)
                            .key_context(OVERLAY_KEY_CONTEXT)
                            .h_full()
                            .max_w_full()
                            .child(self.render_run_panel(entity.clone()))
                            .focus_trap("run-sheet-trap", &self.sheet_focus),
                    ),
                )
            })
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
            .children(self.render_dialog(entity))
    }
}
