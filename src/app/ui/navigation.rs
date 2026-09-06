//! Chat-normal navigation plus a one-shot, focus-preserving global activation.
use gpui::{Context, FocusHandle, KeyDownEvent, Window};
use std::time::{Duration, Instant};

use crate::app::{AppSurface, FarcasterApp, PickerScope};

pub(crate) struct ChatNavigation {
    pub focus: FocusHandle,
    // Remember the chat owner across temporary focus and async session resets.
    pub normal_mode: bool,
    pub pending_key: Option<Prefix>,
    pub activation: Activation,
    pub activation_focus: Option<FocusHandle>,
    pub activation_blur: Option<gpui::Subscription>,
    pub return_shortcut: Option<gpui::Subscription>,
}

mod shortcuts;
pub(crate) use shortcuts::{Command, Prefix, command_key, help_shortcuts};
use shortcuts::{Scroll, normal_command, transcript_scroll};

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
                    "APP · e editor · t terminal · 0–9 sessions · Ctrl+G normal · Esc cancel",
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
        if is_return_chord(key, modifiers, cfg!(target_os = "macos")) {
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
            && let Some(command) = normal_command(key, prefix)
        {
            return ActivatedKey::Command(command);
        }
        // Escape and unknown continuations cancel without leaking into a shell/input.
        ActivatedKey::Cancel
    }
}

fn is_return_chord(key: &str, modifiers: gpui::Modifiers, macos: bool) -> bool {
    key == "g"
        && !modifiers.alt
        && !modifiers.shift
        && !modifiers.function
        && ((modifiers.control && !modifiers.platform)
            || (macos && modifiers.platform && !modifiers.control))
}

impl FarcasterApp {
    pub(in crate::app) fn preferred_chat_focus(&self) -> FocusHandle {
        if self.chat_navigation.normal_mode {
            self.chat_navigation.focus.clone()
        } else {
            self.composer_focus.clone()
        }
    }

    pub(in crate::app) fn initialize_chat_navigation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entity = cx.entity().downgrade();
        let window_id = window.window_handle().window_id();
        // Intercept activation AND its continuations before embedded/input keymaps act.
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
                            ActivatedKey::Pass => return false,
                            ActivatedKey::Pending => {
                                this.chat_navigation.pending_key = None;
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
                            ActivatedKey::Return => this.return_to_chat_normal(window, cx),
                            ActivatedKey::Command(command) => {
                                this.execute_navigation_command(command, false, window, cx);
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
                this.chat_navigation.pending_key = None;
                this.notify_composer(cx);
            }
        })
        .detach();
        cx.on_focus_lost(window, |this, window, cx| {
            // Only repair missing render-tree targets, never ordinary focus changes.
            if !this.chat_navigation.focus.contains_focused(window, cx) {
                this.recover_keyboard_focus(window, cx);
            }
        })
        .detach();
        for (focus, normal_mode) in [
            (&self.chat_navigation.focus, true),
            (&self.composer_focus, false),
        ] {
            cx.on_focus(focus, window, move |this, window, cx| {
                this.chat_navigation.normal_mode = normal_mode;
                this.set_session_shortcuts_visible(normal_mode && window.is_window_active(), cx);
                this.notify_composer(cx);
            })
            .detach();
            cx.on_blur(focus, window, |this, _, cx| {
                this.set_session_shortcuts_visible(false, cx);
                this.chat_navigation.pending_key = None;
                this.notify_composer(cx);
            })
            .detach();
        }
    }

    pub(in crate::app) fn return_to_chat_normal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.chat_navigation.activation.clear();
        self.chat_navigation.normal_mode = true;
        self.chat_navigation.pending_key = None;
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
        // An agent's pending request is not a transient menu: preserve it, without
        // synthesizing a response or cancelling the running agent.
        self.enter_chat_surface(self.chat_navigation.focus.clone(), cx);
        self.chat_navigation.focus.focus(window, cx);
        self.notify_composer(cx);
    }

    pub(in crate::app) fn capture_chat_navigation(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        // Root Tab bindings are intentionally unbound for composer commands.
        // Unhandled Tab still needs to traverse incidental controls explicitly.
        if self.surface == AppSurface::Chat
            && !self.native_workspace_covered_by_overlay()
            && self.chat_navigation.pending_key.is_none()
            && super::focus::traverse_tab(event, None, window, cx)
        {
            return;
        }
        if self.surface != AppSurface::Chat || !self.chat_navigation.focus.is_focused(window) {
            self.chat_navigation.pending_key = None;
            return;
        }
        let scroll = transcript_scroll(key, modifiers, self.chat_navigation.pending_key);
        // Holding g must not synthesize gg; only relative scrolling repeats.
        if event.is_held && !matches!(scroll, Some(Scroll::Lines(_) | Scroll::Pages(_))) {
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        if let Some(scroll) = scroll {
            self.chat_navigation.pending_key = None;
            self.scroll_transcript(scroll, window, cx);
            window.prevent_default();
            cx.stop_propagation();
            self.notify_composer(cx);
            return;
        }
        // Never reinterpret a modified shortcut as a bare normal-mode command.
        if modifiers.modified() {
            self.chat_navigation.pending_key = None;
            self.notify_composer(cx);
            return;
        }
        let pending = std::mem::take(&mut self.chat_navigation.pending_key);
        if pending.is_none()
            && let Some(prefix) = Prefix::from_key(key)
        {
            self.chat_navigation.pending_key = Some(prefix);
        } else if let Some(command) = normal_command(key, pending) {
            self.execute_navigation_command(command, true, window, cx);
        }
        // Unknown continuations and Escape cancel; never replay into a new owner.
        // Tab remains available for deliberate accessible focus traversal.
        if key != "tab" || pending.is_some() {
            window.prevent_default();
            cx.stop_propagation();
        }
        self.notify_composer(cx);
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
            Scroll::Lines(lines) => super::theme::THEME.type_scale.line_reading * lines,
            Scroll::Pages(pages) => list.viewport_height() * pages,
        };
        list.scroll_by(distance, window, self.transcript_view.entity_id());
    }

    fn execute_navigation_command(
        &mut self,
        command: Command,
        normal: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            Command::Composer => self.show_chat_surface(window, cx),
            Command::Editor => self.show_editor_surface(window, cx),
            Command::Terminal => self.show_terminal_surface(window, cx),
            Command::SearchSessions => self.open_picker(PickerScope::Sessions, window, cx),
            Command::RelativeSession(direction) => {
                self.switch_relative_session(direction, window, cx);
                if normal {
                    self.return_to_chat_normal(window, cx);
                }
            }
            Command::Session(number) => {
                if number == 0 {
                    self.switch_to_first_unsubmitted_draft(window, cx);
                } else {
                    self.switch_to_session_number(number, window, cx);
                }
                if normal {
                    self.return_to_chat_normal(window, cx);
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
    fn activation_routes_full_leader_sequences_and_refreshes_timeout() {
        let now = Instant::now();
        for (key, command) in [
            ("j", Command::RelativeSession(1)),
            ("k", Command::RelativeSession(-1)),
        ] {
            let mut state = Activation::default();
            assert_eq!(activated(&mut state, "ctrl-g", now), ActivatedKey::Pending);
            assert_eq!(
                activated(&mut state, "space", now + Duration::from_millis(900)),
                ActivatedKey::Pending
            );
            assert_eq!(
                activated(&mut state, key, now + Duration::from_millis(1500)),
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
        for key in ["escape", "ctrl-2", "z"] {
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
        state.clear(); // session/surface change
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
        // A g prefix cannot become an editor command or survive cancellation/expiry.
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
        assert_eq!(normal_command("e", Some(Prefix::G)), None);
    }

    #[test]
    fn scrolling_respects_leader_and_exact_modifiers() {
        for (key, prefix, expected) in [
            ("g", None, None),
            ("g", Some(Prefix::G), Some(Scroll::Start)),
            ("g", Some(Prefix::Space), None),
            ("G", None, Some(Scroll::End)),
            ("G", Some(Prefix::G), Some(Scroll::End)),
            ("ctrl-g", Some(Prefix::G), None),
            ("alt-g", Some(Prefix::G), None),
            ("ctrl-shift-g", None, None),
            ("j", None, Some(Scroll::Lines(1.0))),
            ("k", None, Some(Scroll::Lines(-1.0))),
            ("ctrl-f", None, Some(Scroll::Pages(1.0))),
            ("ctrl-b", None, Some(Scroll::Pages(-1.0))),
            ("ctrl-d", None, Some(Scroll::Pages(0.5))),
            ("ctrl-u", None, Some(Scroll::Pages(-0.5))),
            ("ctrl-f", Some(Prefix::Space), Some(Scroll::Pages(1.0))),
            ("j", Some(Prefix::Space), None),
            ("k", Some(Prefix::Space), None),
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
    fn normal_and_leader_commands_are_distinct() {
        for (key, prefix, command) in [
            ("i", None, Command::Composer),
            ("a", None, Command::Composer),
            ("/", None, Command::SearchSessions),
            ("e", None, Command::Editor),
            ("t", None, Command::Terminal),
            ("j", Some(Prefix::Space), Command::RelativeSession(1)),
            ("k", Some(Prefix::Space), Command::RelativeSession(-1)),
        ] {
            assert_eq!(normal_command(key, prefix), Some(command));
            assert_eq!(
                normal_command(
                    key,
                    if prefix.is_none() {
                        Some(Prefix::Space)
                    } else {
                        None
                    }
                ),
                None
            );
        }
        assert_eq!(normal_command("escape", Some(Prefix::Space)), None);
    }

    #[test]
    fn session_numbers_are_bare_not_leader_commands() {
        for number in 0..=9 {
            let key = number.to_string();
            assert_eq!(normal_command(&key, None), Some(Command::Session(number)));
            assert_eq!(normal_command(&key, Some(Prefix::Space)), None);
        }
    }

    #[test]
    fn return_chord_is_exact_and_command_alias_is_macos_only() {
        let ctrl = gpui::Modifiers {
            control: true,
            ..Default::default()
        };
        let cmd = gpui::Modifiers {
            platform: true,
            ..Default::default()
        };
        assert!(is_return_chord("g", ctrl, false));
        assert!(is_return_chord("g", ctrl, true));
        assert!(is_return_chord("g", cmd, true));
        assert!(!is_return_chord("g", cmd, false));
        assert!(!is_return_chord("c", ctrl, true));
        assert!(!is_return_chord(
            "g",
            gpui::Modifiers {
                shift: true,
                ..ctrl
            },
            true
        ));
        assert!(!is_return_chord(
            "g",
            gpui::Modifiers { alt: true, ..ctrl },
            true
        ));
    }
}
