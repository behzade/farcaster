mod archive_confirmation;
mod attachments;
mod composer;
mod composer_footer;
#[cfg(test)]
mod composer_tests;
mod delete_confirmation;
mod editor;
mod image_preview;
mod jj_init_confirmation;
mod models;
mod project_trust;
mod regions;
mod run_panel;
mod run_panel_changes;
mod session_groups;
mod session_hover;
mod session_rail;
mod session_rows;
mod settings;
mod shell;
mod surface_switcher;
mod terminal;
pub(crate) mod transcript;
mod usage;
pub(super) mod workgraph;

pub(super) use regions::{
    ComposerView, InactiveSessionRailView, RunPanelView, SessionRailView, TranscriptView,
    WorkGraphDetailView,
};
pub(super) use session_groups::{SessionRailKind, roots_waiting_for_descendants};

use gpui::{
    Context, Focusable as _, InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, StyledImage as _, Window, div, img,
    prelude::FluentBuilder as _,
};
use gpui_base::{Button as BaseButton, TextSelection};
use gpui_component::kbd::Kbd;

use super::{
    APP_INPUT_CONTEXT, AbortRun, AddProject, AppSurface, CloseCurrent, ComposerEscape,
    CurrentCloseTarget, DismissSurface, FarcasterApp, FocusComposer, FocusSessionSearch,
    NATIVE_INPUT_CONTEXT, NewSession, NextSession, PickerBack, PickerScope, PreviousSession,
    ProjectPickerIntent, RemoveProject, ShowActionPicker, ShowEditor, ShowKeybindings,
    ShowTerminal, ShowWorkGraph, SubmitFollowUp, SubmitPrompt, SwitchSession0, SwitchSession1,
    SwitchSession2, SwitchSession3, SwitchSession4, SwitchSession5, SwitchSession6, SwitchSession7,
    SwitchSession8, SwitchSession9, ToggleArchivedSessions, WorkCreateIssue, WorkDismiss,
    WorkFocusSearch, WorkNextIssue, WorkPreviousIssue, current_close_target,
};
pub(crate) const OVERLAY_KEY_CONTEXT: &str = "FarcasterOverlay";

fn session_shortcuts_visible(current: bool, requested: bool, has_text_selection: bool) -> bool {
    if has_text_selection {
        current
    } else {
        requested
    }
}

use crate::{
    assets::AppIcon,
    keyboard::CopySelection,
    layout::{
        layout_mode, shows_left_inline, shows_right_inline, shows_run_sheet_button,
        shows_session_sheet_button,
    },
    primitives::{ButtonTone, FeedbackTone, button, feedback, icon_button, modal},
    protocol::ExtensionUiRequest,
    theme::{THEME, ui_font},
};

impl Render for FarcasterApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = crate::performance::Timing::new("render.root");
        self.resolve_pending_submission(window, cx);
        let native_surface = matches!(self.surface, AppSurface::Editor | AppSurface::Terminal);
        if self.post_render_focus.is_some() {
            cx.defer_in(window, |this, window, cx| {
                this.apply_post_render_focus(window, cx);
            });
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
        if self.pending_dialog_setup {
            if self.dialog_return_focus.is_none() {
                self.dialog_return_focus = window.focused(cx);
            }
            if native_surface {
                self.cover_native_workspace_surface(cx);
            }
            self.pending_dialog_setup = false;
            let dialog = self.extension.dialog.as_ref();
            let prefill = match dialog {
                Some(ExtensionUiRequest::Editor { prefill, .. }) => {
                    prefill.clone().unwrap_or_default()
                }
                _ => String::new(),
            };
            let uses_textarea = matches!(
                dialog,
                Some(ExtensionUiRequest::Input { .. } | ExtensionUiRequest::Editor { .. })
            );
            let input = self.dialog_input.clone();
            let focus = if uses_textarea {
                input.read(cx).focus_handle(cx)
            } else {
                self.dialog_focus.clone()
            };
            cx.defer_in(window, move |_, window, cx| {
                if uses_textarea {
                    input.update(cx, |state, cx| {
                        state.set_value(prefill, window, cx);
                    });
                }
                focus.focus(window, cx);
            });
        }
        if self.native_surface_covered && !self.native_workspace_covered_by_overlay() {
            self.restore_active_native_workspace_surface(window, cx);
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
        if let Some((target, snapshot)) = self.pending_composer_restore.take() {
            cx.defer_in(window, move |this, window, cx| {
                if this.composer_sessions.current_target() == target {
                    this.apply_composer_snapshot(snapshot.clone(), window, cx);
                    this.composer_sessions.capture_current(snapshot);
                }
            });
        }
        let viewport = window.viewport_size();
        let mode = layout_mode(viewport.width);
        let entity = cx.entity().downgrade();
        let key_context = match self.surface {
            AppSurface::Chat | AppSurface::Work => APP_INPUT_CONTEXT,
            AppSurface::Editor | AppSurface::Terminal => NATIVE_INPUT_CONTEXT,
        };
        let work_active = self.surface == AppSurface::Work;
        let has_conversation = !self.selected_draft_is_empty_and_unsubmitted();
        let editable_draft_project = (!has_conversation)
            .then(|| self.editable_draft_project())
            .flatten();
        let editable_draft_harness = (!has_conversation)
            .then(|| self.editable_draft_harness())
            .flatten();
        let chat_main = div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .when(shows_run_sheet_button(mode), |main| {
                main.child(
                    self.render_chat_navigation(shows_session_sheet_button(mode), entity.clone()),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .when(has_conversation, |body| {
                        body.child(self.transcript_view.clone())
                    })
                    .when(!has_conversation, |body| {
                        let heading_entity = entity.clone();
                        body.flex()
                            .items_center()
                            .justify_center()
                            .px(THEME.space.md)
                            .child(
                                div()
                                    .w_full()
                                    .max_w(gpui::px(1080.0))
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .gap(THEME.space.md)
                                    .when_some(
                                        editable_draft_project.zip(editable_draft_harness),
                                        |draft, (project, harness)| {
                                            draft.child(render_draft_heading(
                                                project,
                                                harness,
                                                heading_entity,
                                            ))
                                        },
                                    )
                                    .child(self.composer_view.clone()),
                            )
                    }),
            )
            .when(has_conversation, |main| {
                main.child(self.composer_view.clone())
            })
            .into_any_element();
        let native_surface_covered = native_surface
            && self.native_surface_covered
            && self.native_workspace_covered_by_overlay();
        let main = if native_surface_covered {
            div()
                .size_full()
                .min_h_0()
                .when_some(self.native_surface_snapshot.clone(), |surface, snapshot| {
                    surface.child(img(snapshot).size_full().object_fit(ObjectFit::Fill))
                })
                .into_any_element()
        } else {
            match self.surface {
                AppSurface::Editor => self.render_editor_surface(),
                AppSurface::Terminal => self.render_terminal_workspace(),
                AppSurface::Chat | AppSurface::Work => chat_main,
            }
        };
        let main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(self.render_workspace_bar(entity.clone()))
            .child(div().relative().flex_1().min_h_0().child(main).when(
                native_surface && self.extension.dialog.is_some(),
                |center| {
                    center.child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .child(self.render_composer_request(entity.clone())),
                    )
                },
            ))
            .into_any_element();
        let workgraph_focus = self.workgraph_view.read(cx).focus_handle();
        let picker = self.render_picker(entity.clone(), cx);
        let session_rail_width = self.session_rail_width;
        let run_panel_width = self.run_panel_width;
        div()
            .relative()
            .size_full()
            .bg(THEME.colors.canvas)
            .font(ui_font())
            .key_context(key_context)
            .text_color(THEME.colors.text)
            .text_size(THEME.type_scale.body)
            .on_action(cx.listener(|this, _: &CopySelection, _, cx| {
                crate::keyboard::copy_selection(
                    this.transcript_list.selected_text(),
                    this.composer.read(cx).selected_value().to_string(),
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &DismissSurface, window, cx| {
                this.dismiss_surface(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SubmitFollowUp, window, cx| {
                this.submit_follow_up(window, cx);
            }))
            .on_action(cx.listener(|this, _: &NewSession, window, cx| {
                this.open_picker(
                    PickerScope::Projects(ProjectPickerIntent::NewSession),
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &AddProject, window, cx| {
                this.choose_project_folder(None, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowActionPicker, window, cx| {
                this.open_picker(PickerScope::Actions, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PickerBack, window, cx| {
                this.picker_back(window, cx);
            }))
            .on_action(cx.listener(|this, action: &RemoveProject, window, cx| {
                this.remove_project_from_picker(&action.path, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusSessionSearch, window, cx| {
                this.search_focus.focus(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusComposer, window, cx| {
                if !this.center_surface_switch_blocked() {
                    this.show_chat_surface(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ShowEditor, window, cx| {
                this.show_editor_surface(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowTerminal, window, cx| {
                this.show_terminal_surface(window, cx);
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
                if !value.is_empty() || this.has_composer_attachments() {
                    this.submit(value, this.enter_mode(), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &AbortRun, _, _| {
                if this.snapshot.conversation.running {
                    this.send(crate::runtime::RuntimeCommand::Abort);
                }
            }))
            .on_action(cx.listener(|this, _: &ComposerEscape, _, _| {
                this.handle_composer_escape();
            }))
            .on_action(cx.listener(|this, _: &CloseCurrent, window, cx| {
                if this.surface == AppSurface::Editor {
                    this.close_editor(cx);
                    return;
                }
                if this.surface == AppSurface::Terminal {
                    this.close_terminal(window, cx);
                    return;
                }
                match current_close_target(
                    this.selected_draft.as_deref(),
                    this.snapshot.selected_session.as_deref(),
                ) {
                    CurrentCloseTarget::Draft(id) => this.discard_draft(&id, window, cx),
                    CurrentCloseTarget::Session(path) => {
                        let archived = this
                            .sessions
                            .iter()
                            .find(|session| session.path == path)
                            .is_some_and(|session| session.archived);
                        this.request_session_archive(path, !archived, window, cx);
                    }
                    CurrentCloseTarget::None => {}
                }
            }))
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
            .on_action(cx.listener(|this, _: &WorkCreateIssue, window, cx| {
                this.workgraph_view
                    .update(cx, |view, cx| view.start_create(window, cx));
            }))
            .on_action(cx.listener(|this, _: &WorkDismiss, window, cx| {
                let handled = this
                    .workgraph_view
                    .update(cx, |view, cx| view.dismiss_work_state(window, cx));
                if !handled {
                    this.show_chat_surface(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SwitchSession0, window, cx| {
                this.switch_to_first_unsubmitted_draft(window, cx);
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
                        cfg!(target_os = "macos") && event.modifiers.platform,
                        TextSelection::has_selection(window, cx),
                    );
                    if this.session_shortcuts_visible != visible {
                        this.session_shortcuts_visible = visible;
                        this.notify_session_rail(cx);
                    }
                },
            ))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.runtime_picker_open {
                        this.runtime_picker_open = false;
                        this.notify_composer(cx);
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                this.update_session_rail_resize(event.position.x, cx);
                this.update_run_panel_resize(event.position.x, cx);
            }))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_session_rail_resize(cx);
                    this.finish_run_panel_resize(cx);
                }),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_session_rail_resize(cx);
                    this.finish_run_panel_resize(cx);
                }),
            )
            .child(
                div()
                    .size_full()
                    .flex()
                    .when(shows_left_inline(mode), |shell| {
                        let resize = entity.clone();
                        shell.child(
                            div()
                                .relative()
                                .w(session_rail_width)
                                .min_w(THEME.layout.session_rail_min)
                                .max_w(THEME.layout.session_rail_max)
                                .flex_none()
                                .border_r(THEME.border)
                                .border_color(THEME.colors.border)
                                .child(
                                    self.session_rail_view
                                        .clone()
                                        .cached(gpui::StyleRefinement::default().size_full()),
                                )
                                .child(
                                    div()
                                        .id("session-rail-resize")
                                        .absolute()
                                        .top_0()
                                        .bottom_0()
                                        .right(gpui::px(-4.0))
                                        .w(gpui::px(7.0))
                                        .cursor_col_resize()
                                        .group("session-rail-resize")
                                        .on_mouse_down(
                                            gpui::MouseButton::Left,
                                            move |event, _, cx| {
                                                cx.stop_propagation();
                                                let _ = resize.update(cx, |this, cx| {
                                                    this.begin_session_rail_resize(
                                                        event.position.x,
                                                        cx,
                                                    );
                                                });
                                            },
                                        )
                                        .child(
                                            div()
                                                .ml(gpui::px(3.0))
                                                .w(THEME.border)
                                                .h_full()
                                                .opacity(0.0)
                                                .bg(THEME.colors.muted)
                                                .group_hover("session-rail-resize", |line| {
                                                    line.opacity(1.0)
                                                }),
                                        ),
                                ),
                        )
                    })
                    .child(main)
                    .when(shows_right_inline(mode), |shell| {
                        let resize = entity.clone();
                        shell.child(
                            div()
                                .relative()
                                .w(run_panel_width)
                                .min_w(THEME.layout.run_panel_min)
                                .max_w(THEME.layout.run_panel_max)
                                .flex_none()
                                .border_l(THEME.border)
                                .border_color(THEME.colors.border)
                                .child(if self.workgraph_inspector_issue.is_some() {
                                    self.workgraph_detail_view.clone().into_any_element()
                                } else {
                                    self.run_panel_view
                                        .clone()
                                        .cached(gpui::StyleRefinement::default().size_full())
                                        .into_any_element()
                                })
                                .child(
                                    div()
                                        .id("run-panel-resize")
                                        .absolute()
                                        .top_0()
                                        .bottom_0()
                                        .left(gpui::px(-4.0))
                                        .w(gpui::px(7.0))
                                        .cursor_col_resize()
                                        .group("run-panel-resize")
                                        .on_mouse_down(
                                            gpui::MouseButton::Left,
                                            move |event, _, cx| {
                                                cx.stop_propagation();
                                                let _ = resize.update(cx, |this, cx| {
                                                    this.begin_run_panel_resize(
                                                        event.position.x,
                                                        cx,
                                                    );
                                                });
                                            },
                                        )
                                        .child(
                                            div()
                                                .ml(gpui::px(3.0))
                                                .w(THEME.border)
                                                .h_full()
                                                .opacity(0.0)
                                                .bg(THEME.colors.muted)
                                                .group_hover("run-panel-resize", |line| {
                                                    line.opacity(1.0)
                                                }),
                                        ),
                                ),
                        )
                    }),
            )
            .when_some(picker, |root, picker| root.child(picker))
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
                image_preview::render(self, entity.clone()),
                |root, preview| root.child(preview),
            )
            .when(self.pending_archive.is_some(), |root| {
                root.child(archive_confirmation::render(self, entity.clone()))
            })
            .when(self.pending_delete.is_some(), |root| {
                root.child(delete_confirmation::render(self, entity.clone()))
            })
            .when(self.repository.pending_jj_init.is_some(), |root| {
                root.child(jj_init_confirmation::render(self, entity.clone()))
            })
            .when(self.project_trust_sheet, |root| {
                root.child(project_trust::render(self, entity.clone()))
            })
            .when(self.settings_sheet, |root| {
                root.child(settings::render(self, entity.clone()))
            })
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
                    if self.workgraph_inspector_issue.is_some() {
                        "Node details"
                    } else {
                        "Session details"
                    },
                    &self.sheet_focus,
                    OVERLAY_KEY_CONTEXT,
                    move |window, cx| {
                        let _ = close.update(cx, |this, cx| this.close_sheet(window, cx));
                    },
                    |surface| {
                        surface.h_full().max_w_full().child(
                            if self.workgraph_inspector_issue.is_some() {
                                self.workgraph_detail_view.clone().into_any_element()
                            } else {
                                self.run_panel_view
                                    .clone()
                                    .cached(gpui::StyleRefinement::default().size_full())
                                    .into_any_element()
                            },
                        )
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

fn render_draft_heading(
    project: std::path::PathBuf,
    harness: String,
    entity: gpui::WeakEntity<FarcasterApp>,
) -> impl IntoElement {
    let label = session_rows::project_label(&project);
    let project_entity = entity.clone();
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(THEME.space.sm)
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .text_size(THEME.type_scale.display)
                .text_color(THEME.colors.text)
                .child("What needs doing in ")
                .child(
                    BaseButton::new("draft-project")
                        .accessibility_label(label.clone())
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .text_size(THEME.type_scale.display)
                        .text_color(THEME.colors.accent)
                        .hover(|button| button.text_color(THEME.colors.accent_hover))
                        .active(|button| button.text_color(THEME.colors.accent_active))
                        .focus(|button| button.text_decoration_1())
                        .on_click(move |_, window, cx| {
                            let _ = project_entity.update(cx, |this, cx| {
                                this.open_picker(
                                    PickerScope::Projects(ProjectPickerIntent::ChangeDraft),
                                    window,
                                    cx,
                                );
                            });
                        })
                        .child(label),
                )
                .child("?"),
        )
        .child(
            div().flex().items_center().gap(THEME.space.xs).children(
                crate::agents::backend_statuses()
                    .into_iter()
                    .map(|backend| {
                        let selected = backend.id == harness;
                        let target = backend.id.clone();
                        let entity = entity.clone();
                        let label = if backend.available {
                            backend.name
                        } else {
                            format!("{} unavailable", backend.name)
                        };
                        button(
                            format!("draft-harness-{target}"),
                            label,
                            if selected {
                                ButtonTone::Accent
                            } else {
                                ButtonTone::Quiet
                            },
                            backend.available && !selected,
                            move |window, cx| {
                                let _ = entity.update(cx, |this, cx| {
                                    this.change_draft_harness(target.clone(), window, cx);
                                });
                            },
                        )
                    }),
            ),
        )
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
            #[cfg(not(target_os = "macos"))]
            if let Some(key) = binding.keystroke.strip_prefix("ctrl-") {
                keys = keys.child(Kbd::new(
                    gpui::Keystroke::parse(&format!("super-{key}"))
                        .expect("generated Super shortcut must parse"),
                ));
            }
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
