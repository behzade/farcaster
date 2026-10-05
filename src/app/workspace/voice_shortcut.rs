use gpui::{Context, Keystroke, Window};

use crate::app::{FarcasterApp, workspace::SettingsTab};

const RIGHT_SHIFT: &str = "right-shift";

fn validate(stroke: &Keystroke) -> Result<(), String> {
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
        return Err("Use Right Shift, a modifier with a key, or a function key.".into());
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
                .and_then(|key| Keystroke::parse(key).ok())
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
    pub(in crate::app) fn voice_modifiers_changed(
        &mut self,
        _event: &gpui::ModifiersChangedEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        #[cfg(target_os = "macos")]
        {
            let Some(main) = objc2::MainThreadMarker::new() else {
                return;
            };
            let Some(native) = objc2_app_kit::NSApplication::sharedApplication(main).currentEvent()
            else {
                return;
            };
            // GPUI merges both Shift keys; AppKit retains the device-side flags.
            // NX_DEVICELSHIFTKEYMASK / NX_DEVICERSHIFTKEYMASK from IOLLEvent.h.
            let flags = native.modifierFlags().0;
            let modifiers = _event.modifiers;
            self.voice_right_shift_changed(
                modifiers.shift && flags & 0x4 != 0,
                flags & 0x2 != 0
                    || modifiers.control
                    || modifiers.alt
                    || modifiers.platform
                    || modifiers.function,
                _window,
                _cx,
            );
        }
    }

    pub(super) fn voice_right_shift_changed(
        &mut self,
        down: bool,
        other_modifiers: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_down = std::mem::replace(&mut self.workspace.voice.right_shift_down, down);
        if !self.workspace.voice.capturing_shortcut && self.voice_shortcut() != RIGHT_SHIFT {
            return;
        }
        if other_modifiers {
            self.interrupt_voice_gesture(cx);
        }
        if down && !was_down && !other_modifiers {
            if self.workspace.voice.capturing_shortcut {
                self.workspace.voice.capture_right_shift = true;
            } else {
                self.press_voice_shortcut(RIGHT_SHIFT, window, cx);
            }
        } else if !down && was_down {
            if std::mem::take(&mut self.workspace.voice.capture_right_shift)
                && self.workspace.voice.capturing_shortcut
                && self.overlays.view.settings
                && self.settings.tab == SettingsTab::General
            {
                self.save_voice_shortcut(None, cx);
            }
            if self.workspace.voice.key_down.as_deref() == Some(RIGHT_SHIFT) {
                self.release_voice_shortcut(window, cx);
            }
        }
    }

    pub(in crate::app) fn voice_shortcut(&self) -> &str {
        self.settings
            .voice_shortcut
            .as_deref()
            .unwrap_or(RIGHT_SHIFT)
    }

    pub(super) fn matches_voice_shortcut(&self, stroke: &Keystroke) -> bool {
        let key = self.voice_shortcut();
        key != RIGHT_SHIFT
            && Keystroke::parse(key).is_ok_and(|binding| {
                binding.key == stroke.key && binding.modifiers == stroke.modifiers
            })
    }

    pub(in crate::app) fn voice_shortcut_label(&self) -> String {
        let key = self.voice_shortcut();
        if key == RIGHT_SHIFT {
            return "Right Shift".into();
        }
        Keystroke::parse(key)
            .map(|key| key.to_string())
            .unwrap_or_else(|_| key.to_owned())
    }

    pub(in crate::app) fn begin_voice_shortcut_capture(&mut self, cx: &mut Context<Self>) {
        self.cancel_voice(cx);
        self.workspace.voice.capture_right_shift = false;
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
        if stroke.key == "escape" {
            self.workspace.voice.capturing_shortcut = false;
            self.workspace.voice.settings_error = None;
            cx.notify();
        } else if let Err(error) = validate(stroke) {
            self.workspace.voice.settings_error = Some(error);
            cx.notify();
        } else {
            self.save_voice_shortcut(Some(stroke.unparse()), cx);
        }
        true
    }
}
