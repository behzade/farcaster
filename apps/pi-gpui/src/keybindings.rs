//! Single registry for application keybindings and keyboard-help metadata.

use gpui::KeyBinding;
use gpui_base::actions::{SelectDown, SelectUp};

use crate::app::{
    AbortRun, AddProject, CloseCurrent, ComposerHistoryNext, ComposerHistoryPrevious,
    DismissSurface, FocusComposer, NewSession, NextSession, OVERLAY_KEY_CONTEXT,
    PICKER_KEY_CONTEXT, PickerBack, PreviousSession, QuitApplication, ShowActionPicker,
    ShowKeybindings, ShowWorkGraph, SubmitFollowUp, SubmitPrompt, SwitchSession1, SwitchSession2,
    SwitchSession3, SwitchSession4, SwitchSession5, SwitchSession6, SwitchSession7, SwitchSession8,
    SwitchSession9, ToggleArchivedSessions, WorkCreateIssue, WorkDismiss, WorkFocusSearch,
    WorkNextIssue, WorkPreviousIssue,
};
use crate::app::{WORKGRAPH_KEY_CONTEXT, WORKGRAPH_NAV_KEY_CONTEXT};

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
            "Application",
            "Open action picker",
            "cmd-k",
            ShowActionPicker,
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
            "Close draft or session",
            "cmd-w",
            CloseCurrent,
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
            "Work",
            "Previous node",
            "k",
            WorkPreviousIssue,
            Some(WORKGRAPH_NAV_KEY_CONTEXT)
        ),
        shortcut!(
            "Work",
            "Next node",
            "j",
            WorkNextIssue,
            Some(WORKGRAPH_NAV_KEY_CONTEXT)
        ),
        shortcut!(
            "Work",
            "Search plan",
            "/",
            WorkFocusSearch,
            Some(WORKGRAPH_NAV_KEY_CONTEXT)
        ),
        shortcut!(
            "Work",
            "Add plan node",
            "c",
            WorkCreateIssue,
            Some(WORKGRAPH_NAV_KEY_CONTEXT)
        ),
        shortcut!(
            "Work",
            "Back or clear",
            "escape",
            WorkDismiss,
            Some(WORKGRAPH_KEY_CONTEXT)
        ),
        shortcut!(
            "Application",
            "Open / close project work",
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
        Shortcut {
            section: "Application",
            label: "Previous picker item",
            keystroke: "ctrl-p",
            show_in_help: false,
            binding: KeyBinding::new("ctrl-p", SelectUp, Some("PiPicker > Input")),
        },
        Shortcut {
            section: "Application",
            label: "Next picker item",
            keystroke: "ctrl-n",
            show_in_help: false,
            binding: KeyBinding::new("ctrl-n", SelectDown, Some("PiPicker > Input")),
        },
        Shortcut {
            section: "Application",
            label: "Back in action picker",
            keystroke: "backspace",
            show_in_help: false,
            binding: KeyBinding::new("backspace", PickerBack, Some("PiPicker > Input")),
        },
        Shortcut {
            section: "Application",
            label: "Close action picker",
            keystroke: "escape",
            show_in_help: false,
            binding: KeyBinding::new("escape", DismissSurface, Some(PICKER_KEY_CONTEXT)),
        },
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
            shortcut.label == "Open / close project work" && shortcut.keystroke == "cmd-shift-i"
        }));
        assert!(shortcuts.iter().any(|shortcut| {
            shortcut.label == "Close draft or session" && shortcut.keystroke == "cmd-w"
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
        assert!(shortcuts.iter().any(|shortcut| {
            shortcut.label == "Previous picker item" && shortcut.keystroke == "ctrl-p"
        }));
        assert!(shortcuts.iter().any(|shortcut| {
            shortcut.label == "Next picker item" && shortcut.keystroke == "ctrl-n"
        }));
    }
}
