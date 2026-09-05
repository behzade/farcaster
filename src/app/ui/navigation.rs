//! App navigation owns keys only at its explicit chat-normal focus target.
use gpui::{Context, FocusHandle, KeyDownEvent, Window};

use crate::app::{AppSurface, FarcasterApp, PickerScope};

pub(crate) struct ChatNavigation {
    pub focus: FocusHandle,
    // Remember the chat owner across temporary focus and async session resets.
    pub normal_mode: bool,
    pub leader_pending: bool,
    pub return_shortcut: Option<gpui::Subscription>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Composer,
    Editor,
    Terminal,
    RelativeSession(isize),
    Session(usize),
    SearchSessions,
}

fn normal_command(key: &str, leader: bool) -> Option<Command> {
    match (leader, key) {
        (false, "i" | "a") => Some(Command::Composer),
        (false, "/") => Some(Command::SearchSessions),
        (true, "e") => Some(Command::Editor),
        (true, "t") => Some(Command::Terminal),
        (true, "j") => Some(Command::RelativeSession(1)),
        (true, "k") => Some(Command::RelativeSession(-1)),
        (false, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9") => {
            Some(Command::Session(key.parse().expect("single digit")))
        }
        _ => None,
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
    pub(in crate::app) fn initialize_chat_navigation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entity = cx.entity().downgrade();
        let window_id = window.window_handle().window_id();
        // Reserve the return chord before embedded views or input keymaps act.
        self.chat_navigation.return_shortcut =
            Some(cx.intercept_keystrokes(move |event, window, cx| {
                if window.window_handle().window_id() == window_id
                    && is_return_chord(
                        &event.keystroke.key,
                        event.keystroke.modifiers,
                        cfg!(target_os = "macos"),
                    )
                {
                    let _ = entity.update(cx, |this, cx| this.return_to_chat_normal(window, cx));
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }));
        for (focus, normal_mode) in [
            (&self.chat_navigation.focus, true),
            (&self.composer_focus, false),
        ] {
            cx.on_focus(focus, window, move |this, _, cx| {
                this.chat_navigation.normal_mode = normal_mode;
                this.notify_composer(cx);
            })
            .detach();
            cx.on_blur(focus, window, |this, _, cx| {
                this.chat_navigation.leader_pending = false;
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
        self.chat_navigation.normal_mode = true;
        self.chat_navigation.leader_pending = false;
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
        if self.surface != AppSurface::Chat || !self.chat_navigation.focus.is_focused(window) {
            self.chat_navigation.leader_pending = false;
            return;
        }
        // Never reinterpret a modified shortcut as a bare normal-mode command.
        if modifiers.modified() {
            self.chat_navigation.leader_pending = false;
            self.notify_composer(cx);
            return;
        }
        if event.is_held {
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        let pending = std::mem::take(&mut self.chat_navigation.leader_pending);
        if !pending && key == "space" {
            self.chat_navigation.leader_pending = true;
        } else if let Some(command) = normal_command(key, pending) {
            match command {
                Command::Composer => self.show_chat_surface(window, cx),
                Command::Editor => self.show_editor_surface(window, cx),
                Command::Terminal => self.show_terminal_surface(window, cx),
                Command::SearchSessions => self.open_picker(PickerScope::Sessions, window, cx),
                Command::RelativeSession(direction) => {
                    self.switch_relative_session(direction, window, cx);
                    self.return_to_chat_normal(window, cx);
                }
                Command::Session(number) => {
                    if number == 0 {
                        self.switch_to_first_unsubmitted_draft(window, cx);
                    } else {
                        self.switch_to_session_number(number, window, cx);
                    }
                    self.return_to_chat_normal(window, cx);
                }
            }
        }
        // Unknown continuations and Escape cancel; never replay into a new owner.
        // Tab remains available for deliberate accessible focus traversal.
        if key != "tab" || pending {
            window.prevent_default();
            cx.stop_propagation();
        }
        self.notify_composer(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_and_leader_commands_are_distinct() {
        for (key, leader, command) in [
            ("i", false, Command::Composer),
            ("a", false, Command::Composer),
            ("/", false, Command::SearchSessions),
            ("e", true, Command::Editor),
            ("t", true, Command::Terminal),
            ("j", true, Command::RelativeSession(1)),
            ("k", true, Command::RelativeSession(-1)),
        ] {
            assert_eq!(normal_command(key, leader), Some(command));
            assert_eq!(normal_command(key, !leader), None);
        }
        assert_eq!(normal_command("escape", true), None);
    }

    #[test]
    fn session_numbers_are_bare_not_leader_commands() {
        for number in 0..=9 {
            let key = number.to_string();
            assert_eq!(normal_command(&key, false), Some(Command::Session(number)));
            assert_eq!(normal_command(&key, true), None);
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
