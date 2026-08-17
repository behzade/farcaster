mod attachments;
mod composer;
#[cfg(test)]
mod composer_tests;
mod diff_modal;
mod models;
mod regions;
mod run_panel;
mod session_groups;
mod session_rail;
mod shell;

pub(super) use regions::{ComposerView, RunPanelView, SessionRailView, TranscriptView};
pub(super) use session_groups::session_move_allowed;

use gpui::{
    Context, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::FocusTrapElement as _;

use super::{DismissSurface, PiApp, SubmitFollowUp};
pub(crate) const OVERLAY_KEY_CONTEXT: &str = "PiGpuiOverlay";

use crate::{
    layout::{
        layout_mode, shows_left_inline, shows_right_inline, shows_run_sheet_button,
        shows_session_sheet_button,
    },
    primitives::{FeedbackTone, dialog_backdrop, dialog_surface, feedback},
    protocol::ExtensionUiRequest,
    theme::THEME,
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
        if self.pending_agent_detail_setup {
            self.pending_agent_detail_setup = false;
            let focus = self.agent_detail_focus.clone();
            cx.defer_in(window, move |_, window, cx| focus.focus(window, cx));
        }
        if self.changes.pending_diff_setup {
            self.changes.pending_diff_setup = false;
            let focus = self.changes.diff_focus.clone();
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
                    let snapshot = crate::composer_sessions::ComposerSnapshot::new(text, 0, 0..0);
                    this.apply_composer_snapshot(snapshot.clone(), window, cx);
                    this.composer_sessions.capture_current(snapshot);
                }
            });
        }
        let viewport = window.viewport_size();
        let mode = layout_mode(viewport.width);
        let entity = cx.entity().downgrade();
        let main = div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .when(shows_run_sheet_button(mode), |main| {
                main.child(self.render_navigation(shows_session_sheet_button(mode), entity.clone()))
            })
            .child(
                div().flex_1().min_h_0().child(
                    self.transcript_view
                        .clone()
                        .cached(gpui::StyleRefinement::default().size_full()),
                ),
            )
            .child(self.composer_view.clone());
        div()
            .relative()
            .size_full()
            .bg(THEME.colors.canvas)
            .text_color(THEME.colors.text)
            .text_size(THEME.type_scale.body)
            .on_action(cx.listener(|this, _: &DismissSurface, window, cx| {
                this.dismiss_surface(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SubmitFollowUp, window, cx| {
                this.submit_follow_up(window, cx);
            }))
            .child(
                div()
                    .size_full()
                    .flex()
                    .when(shows_left_inline(mode), |shell| {
                        shell.child(
                            div()
                                .w(THEME.layout.session_rail)
                                .min_w(THEME.layout.session_rail_min)
                                .max_w(THEME.layout.session_rail_max)
                                .flex_none()
                                .border_r(THEME.border)
                                .border_color(THEME.colors.border)
                                .child(
                                    self.session_rail_view
                                        .clone()
                                        .cached(gpui::StyleRefinement::default().size_full()),
                                ),
                        )
                    })
                    .child(main)
                    .when(shows_right_inline(mode), |shell| {
                        shell.child(
                            div()
                                .w(THEME.layout.run_panel)
                                .min_w(THEME.layout.run_panel_min)
                                .max_w(THEME.layout.run_panel_max)
                                .flex_none()
                                .border_l(THEME.border)
                                .border_color(THEME.colors.border)
                                .child(
                                    self.run_panel_view
                                        .clone()
                                        .cached(gpui::StyleRefinement::default().size_full()),
                                ),
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
                            .child(
                                self.session_rail_view
                                    .clone()
                                    .cached(gpui::StyleRefinement::default().size_full()),
                            )
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
                        dialog_surface("run-dialog", "Session details")
                            .track_focus(&self.sheet_focus)
                            .key_context(OVERLAY_KEY_CONTEXT)
                            .h_full()
                            .max_w_full()
                            .child(
                                self.run_panel_view
                                    .clone()
                                    .cached(gpui::StyleRefinement::default().size_full()),
                            )
                            .focus_trap("run-sheet-trap", &self.sheet_focus),
                    ),
                )
            })
            .when(self.agent_detail.is_some(), |root| {
                let close = entity.clone();
                root.child(
                    dialog_backdrop("agent-detail-backdrop", move |window, cx| {
                        let _ = close.update(cx, |this, cx| this.close_agent_detail(window, cx));
                    })
                    .child(
                        dialog_surface("agent-detail-dialog", "Agent")
                            .track_focus(&self.agent_detail_focus)
                            .key_context(OVERLAY_KEY_CONTEXT)
                            .max_w_full()
                            .max_h(THEME.layout.dialog_max_height)
                            .overflow_y_scroll()
                            .children(self.render_agent_detail(entity.clone()))
                            .focus_trap("agent-detail-trap", &self.agent_detail_focus),
                    ),
                )
            })
            .when(self.changes.diff.is_some(), |root| {
                let close = entity.clone();
                root.child(
                    dialog_backdrop("full-diff-backdrop", move |window, cx| {
                        let _ = close.update(cx, |this, cx| this.close_file_diff(window, cx));
                    })
                    .child(
                        dialog_surface("full-diff-dialog", "File diff")
                            .track_focus(&self.changes.diff_focus)
                            .key_context(OVERLAY_KEY_CONTEXT)
                            .w_full()
                            .max_w_full()
                            .h_full()
                            .max_h(gpui::relative(1.0))
                            .overflow_hidden()
                            .child(self.render_diff_modal(entity.clone()))
                            .focus_trap("full-diff-trap", &self.changes.diff_focus),
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
    }
}
