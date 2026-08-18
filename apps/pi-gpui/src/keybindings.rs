//! Single registry for application keybindings and keyboard-help metadata.

use gpui::KeyBinding;

use crate::app::{
    AbortRun, AddProject, ComposerHistoryNext, ComposerHistoryPrevious, DismissSurface,
    FocusComposer, FocusSessionSearch, NewSession, NextSession, OVERLAY_KEY_CONTEXT,
    PreviousSession, QuitApplication, SettleSession, ShowKeybindings, ShowWorkGraph,
    SubmitFollowUp, SubmitPrompt, SwitchSession1, SwitchSession2, SwitchSession3, SwitchSession4,
    SwitchSession5, SwitchSession6, SwitchSession7, SwitchSession8, SwitchSession9,
    ToggleArchivedSessions,
};

pub(crate) struct Shortcut {
    pub section: &'static str,
    pub label: &'static str,
    pub keystroke: &'static str,
    pub show_in_help: bool,
    pub binding: KeyBinding,
}

macro_rules! shortcut {
    ($section:literal, $label:literal, $key:literal, $action:expr, $context:expr) => {
        Shortcut {
            section: $section,
            label: $label,
            keystroke: $key,
            show_in_help: true,
            binding: KeyBinding::new($key, $action, $context),
        }
    };
}

/// Returns the complete application shortcut registry. The same entries install
/// the bindings and render the help modal, so a binding cannot omit its help row.
pub(crate) fn registry() -> Vec<Shortcut> {
    vec![
        shortcut!("Sessions", "Open session 1", "cmd-1", SwitchSession1, None),
        shortcut!("Sessions", "Open session 2", "cmd-2", SwitchSession2, None),
        shortcut!("Sessions", "Open session 3", "cmd-3", SwitchSession3, None),
        shortcut!("Sessions", "Open session 4", "cmd-4", SwitchSession4, None),
        shortcut!("Sessions", "Open session 5", "cmd-5", SwitchSession5, None),
        shortcut!("Sessions", "Open session 6", "cmd-6", SwitchSession6, None),
        shortcut!("Sessions", "Open session 7", "cmd-7", SwitchSession7, None),
        shortcut!("Sessions", "Open session 8", "cmd-8", SwitchSession8, None),
        shortcut!("Sessions", "Open session 9", "cmd-9", SwitchSession9, None),
        shortcut!("Sessions", "New session", "cmd-n", NewSession, None),
        shortcut!("Sessions", "Add project", "cmd-shift-n", AddProject, None),
        shortcut!(
            "Sessions",
            "Search sessions",
            "cmd-k",
            FocusSessionSearch,
            None
        ),
        shortcut!(
            "Sessions",
            "Previous session",
            "cmd-[",
            PreviousSession,
            None
        ),
        shortcut!("Sessions", "Next session", "cmd-]", NextSession, None),
        shortcut!(
            "Sessions",
            "Show archived sessions",
            "cmd-shift-a",
            ToggleArchivedSessions,
            None
        ),
        shortcut!(
            "Sessions",
            "Toggle settled session",
            "cmd-w",
            SettleSession,
            None
        ),
        Shortcut {
            section: "Composer",
            label: "Previous prompt",
            keystroke: "up",
            show_in_help: false,
            binding: KeyBinding::new("up", ComposerHistoryPrevious, Some("PiComposer > Input")),
        },
        Shortcut {
            section: "Composer",
            label: "Next prompt",
            keystroke: "down",
            show_in_help: false,
            binding: KeyBinding::new("down", ComposerHistoryNext, Some("PiComposer > Input")),
        },
        shortcut!("Composer", "Focus composer", "cmd-l", FocusComposer, None),
        shortcut!("Composer", "Send prompt", "cmd-enter", SubmitPrompt, None),
        shortcut!(
            "Composer",
            "Send follow-up",
            "tab",
            SubmitFollowUp,
            Some("PiComposer > Input")
        ),
        shortcut!("Run", "Abort current run", "cmd-.", AbortRun, None),
        shortcut!(
            "Application",
            "Open work graph",
            "cmd-shift-i",
            ShowWorkGraph,
            None
        ),
        shortcut!(
            "Application",
            "Keyboard shortcuts",
            "cmd-?",
            ShowKeybindings,
            None
        ),
        Shortcut {
            section: "Application",
            label: "Keyboard shortcuts",
            keystroke: "cmd-/",
            show_in_help: false,
            binding: KeyBinding::new("cmd-/", ShowKeybindings, None),
        },
        shortcut!(
            "Application",
            "Close dialog",
            "escape",
            DismissSurface,
            Some(OVERLAY_KEY_CONTEXT)
        ),
        shortcut!("Application", "Quit", "cmd-q", QuitApplication, None),
    ]
}

#[cfg(test)]
mod tests {
    use super::registry;

    #[test]
    fn keyboard_help_has_both_question_mark_shortcuts_and_workgraph_navigation() {
        let shortcuts = registry();
        assert!(
            shortcuts
                .iter()
                .any(|shortcut| shortcut.keystroke == "cmd-?")
        );
        assert!(
            shortcuts
                .iter()
                .any(|shortcut| shortcut.keystroke == "cmd-/")
        );
        assert!(shortcuts.iter().any(|shortcut| {
            shortcut.label == "Open work graph" && shortcut.keystroke == "cmd-shift-i"
        }));
        assert!(shortcuts.iter().any(|shortcut| {
            shortcut.label == "Toggle settled session" && shortcut.keystroke == "cmd-w"
        }));
        assert!(
            shortcuts.iter().any(|shortcut| {
                shortcut.label == "Previous prompt" && shortcut.keystroke == "up"
            })
        );
        assert!(
            shortcuts.iter().any(|shortcut| {
                shortcut.label == "Next prompt" && shortcut.keystroke == "down"
            })
        );
    }
}
