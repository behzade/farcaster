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

const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(1);

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
                                return this.handle_chat_scroll(&event.keystroke, window, cx);
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

    fn handle_chat_scroll(
        &mut self,
        keystroke: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.surface != AppSurface::Chat
            || self.native_workspace_covered_by_overlay()
            || !self.composer_region_focus(cx).is_focused(window)
        {
            return false;
        }
        let Some(scroll) = shortcuts::chat_scroll(&keystroke.key, keystroke.modifiers) else {
            return false;
        };
        self.scroll_transcript(scroll, window, cx);
        true
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
            Command::Editor => self.show_editor_surface(window, cx),
            Command::Terminal => self.show_terminal_surface(window, cx),
            Command::SearchSessions => self.open_picker(PickerScope::Sessions, window, cx),
            Command::NewSession => self.open_picker(
                PickerScope::Projects(crate::app::ProjectPickerIntent::NewSession),
                window,
                cx,
            ),
            Command::Close => self.close_current_target(window, cx),
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
mod tests {
    use super::*;

    fn activated(state: &mut Activation, key: &str, now: Instant) -> ActivatedKey {
        let stroke = gpui::Keystroke::parse(key).unwrap();
        state.key(&stroke.key, stroke.modifiers, now)
    }

    #[test]
    fn direct_chat_scroll_accepts_only_the_four_control_chords() {
        for (key, expected) in [
            ("ctrl-f", Some(Scroll::Pages(1.0))),
            ("ctrl-b", Some(Scroll::Pages(-1.0))),
            ("ctrl-u", Some(Scroll::Pages(-0.5))),
            ("ctrl-d", Some(Scroll::Pages(0.5))),
            ("f", None),
            ("b", None),
            ("u", None),
            ("d", None),
            ("j", None),
            ("k", None),
            ("v", None),
            ("ctrl-j", None),
            ("ctrl-k", None),
            ("ctrl-shift-f", None),
            ("ctrl-alt-b", None),
            ("cmd-u", None),
            ("cmd-ctrl-d", None),
        ] {
            let stroke = gpui::Keystroke::parse(key).unwrap();
            assert_eq!(
                shortcuts::chat_scroll(&stroke.key, stroke.modifiers),
                expected,
                "{key}"
            );
        }
    }

    #[test]
    fn activation_is_not_a_leader_and_only_double_g_returns() {
        let now = Instant::now();
        let mut state = Activation::default();
        assert_eq!(activated(&mut state, "ctrl-g", now), ActivatedKey::Pending);
        assert_eq!(
            activated(&mut state, "2", now),
            ActivatedKey::Command(Command::Session(2))
        );
        assert_eq!(activated(&mut state, "2", now), ActivatedKey::Pass);
        assert_eq!(activated(&mut state, "ctrl-g", now), ActivatedKey::Pending);
        assert_eq!(
            activated(&mut state, "e", now),
            ActivatedKey::Command(Command::Editor)
        );
        assert_eq!(activated(&mut state, "ctrl-g", now), ActivatedKey::Pending);
        assert_eq!(activated(&mut state, "ctrl-g", now), ActivatedKey::Return);
        assert!(state.deadline.is_none());
    }

    #[test]
    fn activation_routes_session_keys_without_space() {
        let now = Instant::now();
        for (key, command) in [
            ("j", Command::RelativeSession(1)),
            ("k", Command::RelativeSession(-1)),
        ] {
            let mut state = Activation::default();
            assert_eq!(activated(&mut state, "ctrl-g", now), ActivatedKey::Pending);
            assert_eq!(
                activated(&mut state, key, now + Duration::from_millis(900)),
                ActivatedKey::Command(command)
            );
            assert_eq!(
                activated(&mut state, key, now + Duration::from_millis(1600)),
                ActivatedKey::Pass
            );
        }
    }

    #[test]
    fn timeout_cancellation_and_modified_keys_do_not_leak() {
        let now = Instant::now();
        let mut state = Activation::default();
        activated(&mut state, "ctrl-g", now);
        assert_eq!(
            activated(&mut state, "2", now + ACTIVATION_TIMEOUT),
            ActivatedKey::Pass
        );
        activated(&mut state, "ctrl-g", now);
        assert_eq!(
            activated(&mut state, "ctrl-g", now + ACTIVATION_TIMEOUT),
            ActivatedKey::Pending
        );
        for key in ["escape", "ctrl-2", "z", "space"] {
            state.clear();
            activated(&mut state, "ctrl-g", now);
            assert_eq!(activated(&mut state, key, now), ActivatedKey::Cancel);
            assert_eq!(activated(&mut state, "i", now), ActivatedKey::Pass);
        }
        activated(&mut state, "ctrl-g", now);
        assert_eq!(
            activated(&mut state, "ctrl-f", now),
            ActivatedKey::Scroll(Scroll::Pages(1.0))
        );
        activated(&mut state, "ctrl-g", now);
        state.clear();
        assert_eq!(activated(&mut state, "2", now), ActivatedKey::Pass);
    }

    #[test]
    fn activation_routes_bare_surfaces_and_transcript_boundaries() {
        let now = Instant::now();
        let mut state = Activation::default();
        for (key, command) in [("e", Command::Editor), ("t", Command::Terminal)] {
            activated(&mut state, "ctrl-g", now);
            assert_eq!(
                activated(&mut state, key, now),
                ActivatedKey::Command(command)
            );
            assert_eq!(activated(&mut state, key, now), ActivatedKey::Pass);
        }
        activated(&mut state, "ctrl-g", now);
        assert_eq!(activated(&mut state, "g", now), ActivatedKey::Pending);
        assert_eq!(
            activated(&mut state, "g", now),
            ActivatedKey::Scroll(Scroll::Start)
        );
        activated(&mut state, "ctrl-g", now);
        assert_eq!(
            activated(&mut state, "G", now),
            ActivatedKey::Scroll(Scroll::End)
        );
        for key in ["e", "escape"] {
            activated(&mut state, "ctrl-g", now);
            activated(&mut state, "g", now);
            assert_eq!(activated(&mut state, key, now), ActivatedKey::Cancel);
            assert_eq!(activated(&mut state, "g", now), ActivatedKey::Pass);
        }
        activated(&mut state, "ctrl-g", now);
        activated(&mut state, "g", now);
        assert_eq!(
            activated(&mut state, "g", now + ACTIVATION_TIMEOUT),
            ActivatedKey::Pass
        );
        assert_eq!(shortcuts::activated_command("e", Some(Prefix::G)), None);
    }

    #[test]
    fn scrolling_respects_leader_and_exact_modifiers() {
        for (key, prefix, expected) in [
            ("g", None, None),
            ("g", Some(Prefix::G), Some(Scroll::Start)),
            ("G", None, Some(Scroll::End)),
            ("G", Some(Prefix::G), Some(Scroll::End)),
            ("ctrl-g", Some(Prefix::G), None),
            ("alt-g", Some(Prefix::G), None),
            ("ctrl-shift-g", None, None),
            ("j", None, None),
            ("k", None, None),
            ("ctrl-f", None, Some(Scroll::Pages(1.0))),
            ("ctrl-b", None, Some(Scroll::Pages(-1.0))),
            ("ctrl-d", None, Some(Scroll::Pages(0.5))),
            ("ctrl-u", None, Some(Scroll::Pages(-0.5))),
            ("ctrl-j", None, None),
            ("f", None, None),
            ("ctrl-shift-f", None, None),
            ("cmd-f", None, None),
        ] {
            let stroke = gpui::Keystroke::parse(key).expect("test keystroke");
            assert_eq!(
                transcript_scroll(&stroke.key, stroke.modifiers, prefix),
                expected,
                "{key}, prefix={prefix:?}"
            );
        }
    }

    #[test]
    fn prefix_chord_is_ctrl_g_only() {
        let ctrl = gpui::Modifiers {
            control: true,
            ..Default::default()
        };
        let cmd = gpui::Modifiers {
            platform: true,
            ..Default::default()
        };
        assert!(is_prefix_chord("g", ctrl));
        assert!(!is_prefix_chord("g", cmd));
        assert!(!is_prefix_chord("c", ctrl));
        assert!(!is_prefix_chord(
            "g",
            gpui::Modifiers {
                shift: true,
                ..ctrl
            },
        ));
        assert!(!is_prefix_chord("g", gpui::Modifiers { alt: true, ..ctrl },));
    }

    #[test]
    fn help_lists_ctrl_g_prefix_and_direct_composer_return() {
        let rows = shortcuts::help_shortcuts();
        for key in ["ctrl-f", "ctrl-b", "ctrl-u", "ctrl-d"] {
            assert!(
                rows.iter()
                    .any(|(section, chord, _)| *section == "Chat" && chord == key)
            );
        }
        assert!(
            rows.iter()
                .any(|(section, key, label)| *section == "From anywhere"
                    && key == "ctrl-g"
                    && label.contains("no focus change"))
        );
        assert!(
            rows.iter()
                .any(|(section, key, label)| *section == "From anywhere"
                    && key == "ctrl-g ctrl-g"
                    && *label == "Return to chat composer")
        );
        let has_cmd_g = rows
            .iter()
            .any(|(_, key, label)| key == "cmd-g" && *label == "Focus chat composer");
        assert_eq!(has_cmd_g, cfg!(target_os = "macos"));
    }
}
