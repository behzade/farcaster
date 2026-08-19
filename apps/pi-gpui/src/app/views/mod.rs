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

pub(super) use regions::{
    ComposerView, RunPanelView, SessionRailView, TranscriptView, WorkGraphDetailView,
};

use gpui::{
    Context, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};
use gpui_base::TextSelection;
use gpui_component::kbd::Kbd;

use super::{
    AbortRun, AddProject, AppSurface, CloseCurrent, CurrentCloseTarget, DismissSurface,
    FocusComposer, FocusSessionSearch, NewSession, NextSession, PiApp, PreviousSession,
    ShowKeybindings, ShowWorkGraph, SubmitFollowUp, SubmitPrompt, SwitchSession1, SwitchSession2,
    SwitchSession3, SwitchSession4, SwitchSession5, SwitchSession6, SwitchSession7, SwitchSession8,
    SwitchSession9, ToggleArchivedSessions, WorkCreateIssue, WorkDismiss, WorkFocusSearch,
    WorkNextIssue, WorkPreviousIssue, current_close_target,
};
pub(crate) const OVERLAY_KEY_CONTEXT: &str = "PiGpuiOverlay";

fn session_shortcuts_visible(current: bool, requested: bool, has_text_selection: bool) -> bool {
    if has_text_selection {
        current
    } else {
        requested
    }
}

use crate::{
    app::workgraph::layout::{DETAIL_MAX_WIDTH, DETAIL_MIN_WIDTH, DETAIL_WIDTH},
    layout::{
        layout_mode, shows_left_inline, shows_right_inline, shows_run_sheet_button,
        shows_session_sheet_button,
    },
    primitives::{FeedbackTone, feedback, modal},
    protocol::ExtensionUiRequest,
    theme::THEME,
};

impl Render for PiApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = crate::performance::Timing::new("render.root");
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
        let full_diff_mode = if crate::layout::shows_split_diff(viewport.width - gpui::px(64.0)) {
            crate::app::changes::FullDiffMode::Split
        } else {
            crate::app::changes::FullDiffMode::Unified
        };
        if self.changes.diff.is_some() {
            self.ensure_diff_highlight(full_diff_mode, cx);
        }
        let entity = cx.entity().downgrade();
        let work_active = self.surface == AppSurface::Work;
        let main = if work_active {
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .child(self.render_navigation(
                    shows_session_sheet_button(mode),
                    true,
                    entity.clone(),
                ))
                .child(div().flex_1().min_h_0().child(self.workgraph_view.clone()))
                .into_any_element()
        } else {
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .when(shows_run_sheet_button(mode), |main| {
                    main.child(self.render_navigation(
                        shows_session_sheet_button(mode),
                        false,
                        entity.clone(),
                    ))
                })
                .child(
                    div().flex_1().min_h_0().child(
                        self.transcript_view
                            .clone()
                            .cached(gpui::StyleRefinement::default().size_full()),
                    ),
                )
                .child(self.composer_view.clone())
                .into_any_element()
        };
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
            .on_action(cx.listener(
                |this, _: &CloseCurrent, window, cx| match current_close_target(
                    this.selected_draft.as_deref(),
                    this.snapshot.selected_session.as_deref(),
                ) {
                    CurrentCloseTarget::Draft(id) => this.discard_draft(&id, window, cx),
                    CurrentCloseTarget::Session(path) => {
                        let settled = this
                            .sessions
                            .iter()
                            .find(|session| session.path == path)
                            .is_some_and(|session| session.settled);
                        this.set_session_settled(path, !settled, cx);
                    }
                    CurrentCloseTarget::None => {}
                },
            ))
            .on_action(cx.listener(|this, _: &ShowKeybindings, window, cx| {
                this.open_keybindings_help(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowWorkGraph, window, cx| {
                this.toggle_workgraph_surface(window, cx);
            }))
            .on_action(cx.listener(|this, _: &WorkPreviousIssue, _, cx| {
                this.workgraph_view
                    .update(cx, |view, cx| view.move_selection(-1, cx));
            }))
            .on_action(cx.listener(|this, _: &WorkNextIssue, _, cx| {
                this.workgraph_view
                    .update(cx, |view, cx| view.move_selection(1, cx));
            }))
            .on_action(cx.listener(|this, _: &WorkFocusSearch, window, cx| {
                this.workgraph_view
                    .update(cx, |view, cx| view.focus_search(window, cx));
            }))
            .on_action(cx.listener(|this, _: &WorkCreateIssue, _, cx| {
                this.workgraph_view
                    .update(cx, |view, cx| view.start_create(cx));
            }))
            .on_action(cx.listener(|this, _: &WorkDismiss, window, cx| {
                this.workgraph_view
                    .update(cx, |view, cx| view.dismiss_work_state(window, cx));
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
                |this, event: &gpui::ModifiersChangedEvent, window, cx| {
                    let visible = session_shortcuts_visible(
                        this.session_shortcuts_visible,
                        event.modifiers.platform,
                        TextSelection::has_selection(window, cx),
                    );
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
                                .w(if work_active {
                                    gpui::px(DETAIL_WIDTH)
                                } else {
                                    THEME.layout.run_panel
                                })
                                .min_w(if work_active {
                                    gpui::px(DETAIL_MIN_WIDTH)
                                } else {
                                    THEME.layout.run_panel_min
                                })
                                .max_w(if work_active {
                                    gpui::px(DETAIL_MAX_WIDTH)
                                } else {
                                    THEME.layout.run_panel_max
                                })
                                .flex_none()
                                .border_l(THEME.border)
                                .border_color(THEME.colors.border)
                                .child(if work_active {
                                    self.workgraph_detail_view.clone().into_any_element()
                                } else {
                                    self.run_panel_view
                                        .clone()
                                        .cached(gpui::StyleRefinement::default().size_full())
                                        .into_any_element()
                                }),
                        )
                    }),
            )
            .when(self.keybindings_help, |root| {
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
                            .child(render_keybindings_help())
                    },
                ))
            })
            .when(self.sessions_sheet, |root| {
                let close = entity.clone();
                root.child(modal(
                    "sessions",
                    "Sessions",
                    &self.sheet_focus,
                    OVERLAY_KEY_CONTEXT,
                    move |window, cx| {
                        let _ = close.update(cx, |this, cx| this.close_sheet(window, cx));
                    },
                    |surface| {
                        surface.h_full().max_w_full().child(
                            self.session_rail_view
                                .clone()
                                .cached(gpui::StyleRefinement::default().size_full()),
                        )
                    },
                ))
            })
            .when(self.run_sheet, |root| {
                let close = entity.clone();
                root.child(modal(
                    "run",
                    "Session details",
                    &self.sheet_focus,
                    OVERLAY_KEY_CONTEXT,
                    move |window, cx| {
                        let _ = close.update(cx, |this, cx| this.close_sheet(window, cx));
                    },
                    |surface| {
                        surface.h_full().max_w_full().child(
                            self.run_panel_view
                                .clone()
                                .cached(gpui::StyleRefinement::default().size_full()),
                        )
                    },
                ))
            })
            .when(self.changes.diff.is_some(), |root| {
                let close = entity.clone();
                root.child(modal(
                    "full-diff",
                    "File diff",
                    &self.changes.diff_focus,
                    OVERLAY_KEY_CONTEXT,
                    move |window, cx| {
                        let _ = close.update(cx, |this, cx| this.close_file_diff(window, cx));
                    },
                    |surface| {
                        surface
                            .w_full()
                            .max_w_full()
                            .h_full()
                            .max_h(gpui::relative(1.0))
                            .overflow_hidden()
                            .child(self.render_diff_modal(entity.clone(), full_diff_mode))
                    },
                ))
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

#[cfg(test)]
mod tests {
    use super::session_shortcuts_visible;

    #[test]
    fn command_modifier_does_not_change_shortcuts_during_text_selection() {
        assert!(!session_shortcuts_visible(false, true, true));
        assert!(session_shortcuts_visible(true, false, true));
        assert!(session_shortcuts_visible(false, true, false));
    }
}
