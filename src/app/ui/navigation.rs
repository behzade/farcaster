use gpui::{Context, FocusHandle, KeyDownEvent, Window};
use std::time::{Duration, Instant};

use crate::app::{AppSurface, FarcasterApp, PickerScope};

pub(crate) struct ChatNavigation {
    pub focus: FocusHandle,
    pub activation: Activation,
    pub activation_focus: Option<FocusHandle>,
    pub activation_blur: Option<gpui::Subscription>,
    pub return_shortcut: Option<gpui::Subscription>,
}

mod shortcuts;
pub(crate) use shortcuts::{Command, command_key, help_shortcuts};
use shortcuts::{Prefix, Scroll, transcript_scroll};

const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(crate) struct Activation {
    deadline: Option<Instant>,
    prefix: Option<Prefix>,
}

#[derive(Debug, PartialEq)]
enum ActivatedKey {
    Pass,
    Pending,
    Cancel,
    Return,
    Command(Command),
    Scroll(Scroll),
}

impl Activation {
    pub(in crate::app) fn hint(&self) -> Option<&'static str> {
        self.deadline
            .filter(|deadline| Instant::now() < *deadline)
            .map(|_| {
                self.prefix.map(Prefix::hint).unwrap_or(
                    "APP · e editor · t terminal · 0–9 sessions · Ctrl+G composer · Esc cancel",
                )
            })
    }

    pub(in crate::app) fn clear(&mut self) {
        self.deadline = None;
        self.prefix = None;
    }

    fn key(&mut self, key: &str, modifiers: gpui::Modifiers, now: Instant) -> ActivatedKey {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.clear();
        }
        if is_prefix_chord(key, modifiers) {
            if self.deadline.is_some() {
                self.clear();
                return ActivatedKey::Return;
            }
            self.deadline = Some(now + ACTIVATION_TIMEOUT);
            return ActivatedKey::Pending;
        }
        if self.deadline.is_none() {
            return ActivatedKey::Pass;
        }
        if self.prefix.is_none()
            && !modifiers.modified()
            && let Some(prefix) = Prefix::from_key(key)
        {
            self.prefix = Some(prefix);
            self.deadline = Some(now + ACTIVATION_TIMEOUT);
            return ActivatedKey::Pending;
        }
        let prefix = self.prefix;
        self.clear();
        if let Some(scroll) = transcript_scroll(key, modifiers, prefix) {
            return ActivatedKey::Scroll(scroll);
        }
        if prefix.is_none()
            && key == "n"
            && modifiers
                == (gpui::Modifiers {
                    shift: true,
                    ..Default::default()
                })
        {
            return ActivatedKey::Command(Command::StartCodeTask);
        }
        if !modifiers.modified()
            && let Some(command) = shortcuts::activated_command(key, prefix)
        {
            return ActivatedKey::Command(command);
        }
        ActivatedKey::Cancel
    }
}

fn is_prefix_chord(key: &str, modifiers: gpui::Modifiers) -> bool {
    key == "g"
        && modifiers.control
        && !modifiers.platform
        && !modifiers.alt
        && !modifiers.shift
        && !modifiers.function
}

impl FarcasterApp {
    pub(in crate::app) fn chat_composer_focus(&self, cx: &gpui::App) -> FocusHandle {
        self.composer_region_focus(cx)
    }

    pub(in crate::app) fn initialize_chat_navigation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entity = cx.entity().downgrade();
        let window_id = window.window_handle().window_id();
        self.chat_navigation.return_shortcut =
            Some(cx.intercept_keystrokes(move |event, window, cx| {
                if window.window_handle().window_id() != window_id {
                    return;
                }
                let consumed = entity
                    .update(cx, |this, cx| {
                        if this.chat_navigation.activation_focus != window.focused(cx) {
                            this.chat_navigation.activation.clear();
                        }
                        let result = this.chat_navigation.activation.key(
                            &event.keystroke.key,
                            event.keystroke.modifiers,
                            Instant::now(),
                        );
                        match result {
                            ActivatedKey::Pass => {
                                return false;
                            }
                            ActivatedKey::Pending => {
                                this.chat_navigation.activation_focus = window.focused(cx);
                                this.chat_navigation.activation_blur =
                                    this.chat_navigation.activation_focus.clone().map(|focus| {
                                        cx.on_blur(&focus, window, |this, _, cx| {
                                            this.chat_navigation.activation.clear();
                                            this.notify_composer(cx);
                                        })
                                    });
                                let deadline = this.chat_navigation.activation.deadline;
                                cx.spawn(async move |weak, cx| {
                                    cx.background_executor().timer(ACTIVATION_TIMEOUT).await;
                                    let _ = weak.update(cx, |this, cx| {
                                        if this.chat_navigation.activation.deadline == deadline {
                                            this.chat_navigation.activation.clear();
                                            this.notify_composer(cx);
                                        }
                                    });
                                })
                                .detach();
                            }
                            ActivatedKey::Return => this.return_to_chat_composer(window, cx),
                            ActivatedKey::Command(command) => {
                                this.execute_navigation_command(command, window, cx);
                            }
                            ActivatedKey::Scroll(scroll) => {
                                this.scroll_transcript(scroll, window, cx)
                            }
                            ActivatedKey::Cancel => {}
                        }
                        this.notify_composer(cx);
                        true
                    })
                    .unwrap_or(false);
                if consumed {
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }));
        cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.chat_navigation.activation.clear();
                this.notify_composer(cx);
            }
        })
        .detach();
        cx.on_focus_lost(window, |this, window, cx| {
            if !this.chat_navigation.focus.contains_focused(window, cx) {
                this.recover_keyboard_focus(window, cx);
            }
        })
        .detach();
        cx.on_focus(&self.composer_focus, window, |this, _, cx| {
            this.notify_composer(cx);
        })
        .detach();
    }

    pub(in crate::app) fn return_to_chat_composer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.chat_navigation.activation.clear();
        if self.repository.edits.pending.is_some() {
            self.close_repository_edit(window, cx);
            if self.repository.edits.pending.is_some() {
                return;
            }
        }
        if self.send_to_chat.is_some() {
            self.close_send_to_chat(window, cx);
        }
        if self.image_preview.is_some() {
            self.close_image_preview(window, cx);
        }
        if self.repository.pending_jj_init.is_some() {
            self.close_jj_init_confirmation(window, cx);
        }
        if self.picker.is_some() {
            self.close_picker(window, cx);
        }
        self.pending_archive = None;
        self.pending_delete = None;
        self.session_import = None;
        if self.overlays.project_trust {
            self.dismiss_project_trust(window, cx);
        }
        if self.overlays.sessions
            || self.overlays.run
            || self.overlays.keybindings
            || self.overlays.settings
        {
            self.close_sheet(window, cx);
        }
        let focus = self.chat_composer_focus(cx);
        self.enter_chat_surface(focus.clone(), cx);
        focus.focus(window, cx);
        self.notify_transcript(cx);
        self.notify_composer(cx);
    }

    pub(in crate::app) fn capture_chat_navigation(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.surface == AppSurface::Chat && !self.native_workspace_covered_by_overlay() {
            super::focus::traverse_tab(event, None, window, cx);
        }
    }

    fn scroll_transcript(&mut self, scroll: Scroll, window: &mut Window, cx: &mut Context<Self>) {
        let list = &self.transcript_view.read(cx).list;
        let distance = match scroll {
            Scroll::Start => {
                self.transcript_view.update(cx, |transcript, cx| {
                    transcript.list.scroll_to_start();
                    transcript.following = false;
                    cx.notify();
                });
                return;
            }
            Scroll::End => {
                self.jump_to_latest(cx);
                return;
            }
            Scroll::Pages(pages) => list.viewport_height() * pages,
        };
        list.scroll_by(distance, window, self.transcript_view.entity_id());
    }

    fn execute_navigation_command(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            Command::Actions => window.dispatch_action(Box::new(crate::app::ShowActionPicker), cx),
            Command::AddProject => window.dispatch_action(Box::new(crate::app::AddProject), cx),
            Command::Sandbox => window.dispatch_action(Box::new(crate::app::SetSandbox), cx),
            Command::Harness => window.dispatch_action(Box::new(crate::app::SetHarness), cx),
            Command::Runtime => window.dispatch_action(Box::new(crate::app::SetRuntime), cx),
            Command::RestoreSession => {
                window.dispatch_action(Box::new(crate::app::RestoreSession), cx)
            }
            Command::Editor => self.show_editor_surface(window, cx),
            Command::SendToChat => self.open_send_to_chat(window, cx),
            Command::StartCodeTask => self.start_task_from_code(window, cx),
            Command::TranscriptScratch => self.open_transcript_scratch(window, cx),
            Command::Terminal => self.show_terminal_surface(window, cx),
            Command::SearchSessions => self.open_picker(PickerScope::Sessions, window, cx),
            Command::NewSession => self.open_picker(
                PickerScope::Projects(crate::app::ProjectPickerIntent::NewSession),
                window,
                cx,
            ),
            Command::Close => self.close_current_target(window, cx),
            Command::Quit => window.dispatch_action(Box::new(crate::app::QuitApplication), cx),
            Command::RelativeSession(direction) => {
                self.switch_relative_session(direction, window, cx);
            }
            Command::Session(number) => {
                if number == 0 {
                    self.switch_to_first_unsubmitted_draft(window, cx);
                } else {
                    self.switch_to_session_number(number, window, cx);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;
