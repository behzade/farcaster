use gpui::{Context, Keystroke, Modifiers, Window};

use crate::app::{FarcasterApp, workspace::SettingsTab};

#[cfg(target_os = "macos")]
pub(super) const DEFAULT_SHORTCUT: &str = "ctrl-cmd";
#[cfg(not(target_os = "macos"))]
pub(super) const DEFAULT_SHORTCUT: &str = "ctrl-super";

pub(super) fn modifier_binding(key: &str) -> Option<Modifiers> {
    match key {
        "ctrl-cmd" | "ctrl-super" => Some(Modifiers {
            control: true,
            platform: true,
            ..Default::default()
        }),
        "cmd-alt" => Some(Modifiers {
            platform: true,
            alt: true,
            ..Default::default()
        }),
        "ctrl-alt" => Some(Modifiers {
            control: true,
            alt: true,
            ..Default::default()
        }),
        _ => None,
    }
}

#[derive(Default)]
pub(super) struct ModifierShortcut {
    modifiers: Modifiers,
    keys: std::collections::HashSet<String>,
    capture_blocked: bool,
    capture: Option<&'static str>,
}

impl ModifierShortcut {
    pub(super) fn reset(&mut self, modifiers: Modifiers) {
        *self = Self {
            modifiers,
            capture_blocked: modifiers.modified(),
            ..Default::default()
        };
    }

    pub(super) fn key_down(&mut self, key: &str) {
        self.keys.insert(key.to_owned());
        self.capture_blocked = true;
        self.capture = None;
    }

    pub(super) fn key_up(&mut self, key: &str) {
        self.keys.remove(key);
        if self.keys.is_empty() && !self.modifiers.modified() {
            self.capture_blocked = false;
        }
    }

    // Commit on release so a following key can still form Ctrl+Alt+V, etc.
    fn capture(&mut self) -> Option<&'static str> {
        if let Some(key) = self.capture {
            let required = modifier_binding(key).unwrap();
            if self.modifiers == required {
                return None;
            }
            self.capture = None;
            self.capture_blocked = self.modifiers.modified();
            return self.modifiers.is_subset_of(&required).then_some(key);
        }
        if !self.capture_blocked && self.keys.is_empty() {
            self.capture = [DEFAULT_SHORTCUT, "cmd-alt", "ctrl-alt"]
                .into_iter()
                .find(|key| modifier_binding(key) == Some(self.modifiers));
        }
        None
    }
}

fn validate(stroke: &Keystroke, cx: &gpui::App) -> Result<(), String> {
    let function_key = stroke
        .key
        .strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n));
    if !function_key
        && !stroke.modifiers.control
        && !stroke.modifiers.alt
        && !stroke.modifiers.platform
    {
        return Err("Use a modifier with a key, or a function key.".into());
    }
    if crate::app::ui::navigation::is_prefix_chord(&stroke.key, stroke.modifiers) {
        return Err("Ctrl+G is used for app navigation.".into());
    }
    for command in crate::app::ui::keybindings::registry() {
        if command.bindings.iter().any(|binding| {
            binding
                .keystroke
                .split_whitespace()
                .next()
                .and_then(|key| Keystroke::parse_with_os(key, cx.platform_info().os).ok())
                .is_some_and(|key| key.key == stroke.key && key.modifiers == stroke.modifiers)
        }) {
            return Err(format!(
                "Already used for {}.",
                command.label.to_lowercase()
            ));
        }
    }
    Ok(())
}

impl FarcasterApp {
    pub(in crate::app) fn voice_shortcut(&self) -> &str {
        self.settings
            .voice_shortcut
            .as_deref()
            // Retire the old modifier-only binding on every platform.
            .filter(|key| *key != "right-shift")
            .unwrap_or(DEFAULT_SHORTCUT)
    }

    pub(super) fn matches_voice_shortcut(&self, stroke: &Keystroke, cx: &gpui::App) -> bool {
        let key = self.voice_shortcut();
        modifier_binding(key).is_none()
            && Keystroke::parse_with_os(key, cx.platform_info().os).is_ok_and(|binding| {
                binding.key == stroke.key && binding.modifiers == stroke.modifiers
            })
    }

    pub(in crate::app) fn voice_shortcut_label(&self, cx: &gpui::App) -> String {
        let key = self.voice_shortcut();
        Keystroke::parse_with_os(key, cx.platform_info().os)
            .map(|key| key.display(cx.platform_info().os).to_string())
            .unwrap_or_else(|_| key.to_owned())
    }

    pub(in crate::app) fn voice_modifiers_changed(
        &mut self,
        event: &gpui::ModifiersChangedEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let held = event.modifiers;
        if !window.is_window_active() {
            self.cancel_voice(cx);
            self.workspace.voice.release_keys(held);
            return;
        }
        let binding = modifier_binding(self.voice_shortcut());
        let state = &mut self.workspace.voice.modifier_shortcut;
        let previous = state.modifiers;
        state.modifiers = held;
        if !held.modified() && state.keys.is_empty() {
            state.capture_blocked = false;
        }
        if self.workspace.voice.capturing_shortcut {
            if !self.overlays.view.settings || self.settings.tab != SettingsTab::General {
                self.workspace.voice.capturing_shortcut = false;
                state.capture = None;
                return;
            }
            if let Some(key) = state.capture() {
                self.save_voice_shortcut(Some(key.into()), cx);
            }
            return;
        }
        let Some(required) = binding else {
            return;
        };
        let was_down = previous == required;
        let is_down = held == required;
        if is_down == was_down {
            return;
        }
        if is_down {
            if state.keys.is_empty() {
                let key = self.voice_shortcut().to_owned();
                self.press_voice_shortcut(&key, window, cx);
            }
        } else if held.is_subset_of(&required) {
            self.release_voice_shortcut(cx);
        } else {
            self.interrupt_voice_gesture(cx);
            self.workspace.voice.key_down = None;
        }
    }

    pub(in crate::app) fn begin_voice_shortcut_capture(&mut self, cx: &mut Context<Self>) {
        self.cancel_voice(cx);
        let state = &mut self.workspace.voice.modifier_shortcut;
        state.capture = None;
        state.capture_blocked = state.modifiers.modified() || !state.keys.is_empty();
        self.workspace.voice.capturing_shortcut = !self.workspace.voice.capturing_shortcut;
        self.workspace.voice.settings_error = None;
        cx.notify();
    }

    pub(in crate::app) fn reset_voice_shortcut(&mut self, cx: &mut Context<Self>) {
        self.save_voice_shortcut(None, cx);
    }

    fn save_voice_shortcut(&mut self, shortcut: Option<String>, cx: &mut Context<Self>) {
        match crate::app::persistence::open()
            .and_then(|store| store.save_voice_shortcut(shortcut.as_deref()))
        {
            Ok(()) => {
                self.settings.voice_shortcut = shortcut;
                self.workspace.voice.capturing_shortcut = false;
                self.workspace.voice.settings_error = None;
            }
            Err(error) => self.workspace.voice.settings_error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn capture_voice_shortcut(
        &mut self,
        stroke: &Keystroke,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.workspace.voice.capturing_shortcut {
            return false;
        }
        if !self.overlays.view.settings || self.settings.tab != SettingsTab::General {
            self.workspace.voice.capturing_shortcut = false;
            return false;
        }
        if let Err(error) = validate(stroke, cx) {
            self.workspace.voice.settings_error = Some(error);
            cx.notify();
        } else {
            self.save_voice_shortcut(Some(stroke.unparse(cx.platform_info().os)), cx);
        }
        true
    }
}
