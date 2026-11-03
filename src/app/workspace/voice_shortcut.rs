use gpui::{Context, Keystroke, Window};

use crate::app::{FarcasterApp, workspace::SettingsTab};

const DEFAULT_SHORTCUT: &str = "ctrl-'";

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
        let matches = |key| {
            Keystroke::parse_with_os(key, cx.platform_info().os).is_ok_and(|binding| {
                binding.key == stroke.key && binding.modifiers == stroke.modifiers
            })
        };
        matches(key) || (cfg!(target_os = "macos") && key == DEFAULT_SHORTCUT && matches("cmd-'"))
    }

    pub(in crate::app) fn voice_shortcut_label(&self, cx: &gpui::App) -> String {
        let key = self.voice_shortcut();
        Keystroke::parse_with_os(key, cx.platform_info().os)
            .map(|key| key.display(cx.platform_info().os).to_string())
            .unwrap_or_else(|_| key.to_owned())
    }

    pub(in crate::app) fn begin_voice_shortcut_capture(&mut self, cx: &mut Context<Self>) {
        self.cancel_voice(cx);
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
