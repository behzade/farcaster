mod attachments;
mod composer;
#[cfg(test)]
mod composer_tests;
mod diff_modal;
mod models;
mod regions;
mod run_panel;
mod run_panel_changes;
mod session_groups;
mod session_rail;
mod session_rows;
mod shell;

pub(super) use regions::{ComposerView, RunPanelView, SessionRailView, TranscriptView};
pub(super) use session_groups::session_move_allowed;

use gpui::{
    Context, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::{FocusTrapElement as _, kbd::Kbd};

use super::{
    AbortRun, AddProject, DismissSurface, FocusComposer, FocusSessionSearch, NewSession,
    NextSession, PiApp, PreviousSession, ShowKeybindings, ShowWorkGraph, SubmitFollowUp,
    SubmitPrompt, SwitchSession1, SwitchSession2, SwitchSession3, SwitchSession4, SwitchSession5,
    SwitchSession6, SwitchSession7, SwitchSession8, SwitchSession9, ToggleArchivedSessions,
};
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
        if self.pending_session_title_focus {
            self.pending_session_title_focus = false;
            let focus = self.session_title_input.read(cx).focus_handle(cx);
            cx.defer_in(window, move |_, window, cx| focus.focus(window, cx));
        }
        if self.pending_sheet_setup {
            self.pending_sheet_setup = false;
            let focus = self.sheet_focus.clone();
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
            .on_action(cx.listener(|this, _: &NewSession, window, cx| {
                this.new_session(this.project.clone(), window, cx);
            }))
            .on_action(cx.listener(|this, _: &AddProject, window, cx| {
                this.choose_project_folder(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusSessionSearch, window, cx| {
                this.search_focus.focus(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusComposer, window, cx| {
                this.composer_focus.focus(window, cx);
            }))
            .on_action(cx.listener(|this, _: &PreviousSession, window, cx| {
                this.switch_relative_session(-1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextSession, window, cx| {
                this.switch_relative_session(1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleArchivedSessions, _, cx| {
                this.archived_sessions_expanded = !this.archived_sessions_expanded;
                this.notify_session_rail(cx);
            }))
            .on_action(cx.listener(|this, _: &SubmitPrompt, window, cx| {
                let value = this.composer.read(cx).value().trim().to_owned();
                if !value.is_empty() || this.has_composer_images() {
                    this.submit(value, this.enter_mode(), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &AbortRun, _, _| {
                if this.snapshot.conversation.running {
                    this.send(crate::runtime::RuntimeCommand::Abort);
                }
            }))
            .on_action(cx.listener(|this, _: &ShowKeybindings, window, cx| {
                this.open_keybindings_help(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowWorkGraph, window, cx| {
                this.open_workgraph_sheet(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwitchSession1, window, cx| {
                this.switch_to_session_number(1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwitchSession2, window, cx| {
                this.switch_to_session_number(2, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwitchSession3, window, cx| {
                this.switch_to_session_number(3, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwitchSession4, window, cx| {
                this.switch_to_session_number(4, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwitchSession5, window, cx| {
                this.switch_to_session_number(5, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwitchSession6, window, cx| {
                this.switch_to_session_number(6, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwitchSession7, window, cx| {
                this.switch_to_session_number(7, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwitchSession8, window, cx| {
                this.switch_to_session_number(8, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SwitchSession9, window, cx| {
                this.switch_to_session_number(9, window, cx);
            }))
            .on_modifiers_changed(cx.listener(
                |this, event: &gpui::ModifiersChangedEvent, _, cx| {
                    let visible = event.modifiers.platform;
                    if this.session_shortcuts_visible != visible {
                        this.session_shortcuts_visible = visible;
                        this.notify_session_rail(cx);
                    }
                },
            ))
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
            .when(self.keybindings_help, |root| {
                let close = entity.clone();
                root.child(
                    dialog_backdrop("keybindings-help-backdrop", move |window, cx| {
                        let _ = close.update(cx, |this, cx| this.close_sheet(window, cx));
                    })
                    .child(
                        dialog_surface("keybindings-help", "Keyboard shortcuts")
                            .track_focus(&self.sheet_focus)
                            .key_context(OVERLAY_KEY_CONTEXT)
                            .w(gpui::px(520.0))
                            .max_w_full()
                            .child(render_keybindings_help())
                            .focus_trap("keybindings-help-trap", &self.sheet_focus),
                    ),
                )
            })
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
            .when(self.workgraph_sheet, |root| {
                let close = entity.clone();
                root.child(
                    dialog_backdrop("workgraph-sheet", move |window, cx| {
                        let _ = close.update(cx, |this, cx| this.close_sheet(window, cx));
                    })
                    .child(
                        dialog_surface("workgraph-dialog", "Work graph")
                            .track_focus(&self.sheet_focus)
                            .key_context(OVERLAY_KEY_CONTEXT)
                            .w_full()
                            .max_w(gpui::px(1_080.0))
                            .h_full()
                            .max_h(gpui::relative(0.92))
                            .overflow_hidden()
                            .child(self.workgraph_view.clone())
                            .focus_trap("workgraph-sheet-trap", &self.sheet_focus),
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
                            .child(self.render_diff_modal(
                                entity.clone(),
                                if crate::layout::shows_split_diff(viewport.width - gpui::px(64.0))
                                {
                                    crate::app::changes::FullDiffMode::Split
                                } else {
                                    crate::app::changes::FullDiffMode::Unified
                                },
                            ))
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

fn render_keybindings_help() -> impl IntoElement {
    let shortcuts = crate::keybindings::registry();
    let mut content = div()
        .flex()
        .flex_col()
        .gap(THEME.space.md)
        .p(THEME.space.md)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(THEME.space.xs)
                .pb(THEME.space.sm)
                .border_b(THEME.border)
                .border_color(THEME.colors.border)
                .child(
                    div()
                        .text_size(THEME.type_scale.display)
                        .text_color(THEME.colors.text)
                        .child("Keyboard shortcuts"),
                )
                .child(
                    div()
                        .text_size(THEME.type_scale.body_small)
                        .text_color(THEME.colors.muted)
                        .child("Navigate Pi without leaving the keyboard."),
                ),
        );
    let mut current_section = "";
    let mut section = None;

    for shortcut in shortcuts.iter().filter(|shortcut| shortcut.show_in_help) {
        if shortcut.section != current_section {
            if let Some(previous) = section.take() {
                content = content.child(previous);
            }
            current_section = shortcut.section;
            section = Some(
                div().flex().flex_col().gap(THEME.space.xs).child(
                    div()
                        .mb(THEME.space.xs)
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.accent)
                        .child(current_section),
                ),
            );
        }

        let mut keys = div().flex().items_center().gap(THEME.space.xs);
        for binding in shortcuts.iter().filter(|candidate| {
            candidate.section == shortcut.section && candidate.label == shortcut.label
        }) {
            keys = keys.child(Kbd::new(
                gpui::Keystroke::parse(binding.keystroke).expect("registered shortcut must parse"),
            ));
        }
        section = section.map(|section| {
            section.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(THEME.space.md)
                    .min_h(THEME.controls.utility_row)
                    .px(THEME.space.sm)
                    .py(THEME.space.xs)
                    .rounded(THEME.radius)
                    .bg(THEME.colors.surface)
                    .child(shortcut.label)
                    .child(keys),
            )
        });
    }

    if let Some(section) = section {
        content = content.child(section);
    }
    content
}
