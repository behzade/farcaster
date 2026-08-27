//! Single registry for application keybindings and keyboard-help metadata.

use crate::app::{
    AbortRun, AddProject, CloseCurrent, ComposerEscape, ComposerHistoryNext,
    ComposerHistoryPrevious, DismissSurface, FocusComposer, NewSession, NextSession,
    OVERLAY_KEY_CONTEXT, PICKER_KEY_CONTEXT, PickerBack, PreviousSession, QuitApplication,
    ShowActionPicker, ShowEditor, ShowKeybindings, ShowTerminal, ShowWorkGraph, SubmitFollowUp,
    SubmitPrompt, SwitchSession0, SwitchSession1, SwitchSession2, SwitchSession3, SwitchSession4,
    SwitchSession5, SwitchSession6, SwitchSession7, SwitchSession8, SwitchSession9,
    ToggleArchivedSessions, WorkCreateIssue, WorkDismiss, WorkFocusSearch, WorkNextIssue,
    WorkPreviousIssue,
};
use crate::app::{WORKGRAPH_KEY_CONTEXT, WORKGRAPH_NAV_KEY_CONTEXT};
use crate::keyboard::CopySelection;
use crate::transcript_list::TRANSCRIPT_SELECTION_KEY_CONTEXT;
use gpui::{KeyBinding, Unbind};
use gpui_base::actions::{SelectDown, SelectUp};

#[cfg(target_os = "macos")]
pub(crate) const PRIMARY_MODIFIER: &str = "cmd";
#[cfg(not(target_os = "macos"))]
pub(crate) const PRIMARY_MODIFIER: &str = "ctrl";

pub(crate) const fn primary_key(macos: &'static str, non_macos: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        macos
    } else {
        non_macos
    }
}

pub(crate) struct Shortcut {
    pub section: &'static str,
    pub label: &'static str,
    pub keystroke: &'static str,
    pub show_in_help: bool,
    pub binding: KeyBinding,
}

macro_rules! primary {
    ($key:literal) => {
        primary_key(concat!("cmd-", $key), concat!("ctrl-", $key))
    };
}

macro_rules! shortcut {
    ($section:literal, $label:literal, $key:expr, $action:expr, $context:expr) => {
        Shortcut {
            section: $section,
            label: $label,
            keystroke: $key,
            show_in_help: true,
            binding: KeyBinding::new($key, $action, $context),
        }
    };
}

macro_rules! primary_shortcut {
    ($section:literal, $label:literal, $key:literal, $action:expr) => {
        shortcut!($section, $label, primary!($key), $action, None)
    };
}

/// Returns application shortcuts plus overrides for dependency-owned defaults.
pub(crate) fn bindings() -> Vec<KeyBinding> {
    registry()
        .into_iter()
        .map(|shortcut| shortcut.binding)
        .chain([
            KeyBinding::new("tab", Unbind("root::Tab".into()), Some("Root")),
            KeyBinding::new("shift-tab", Unbind("root::TabPrev".into()), Some("Root")),
        ])
        .collect()
}

/// Returns the user-facing shortcut registry used by keyboard help.
pub(crate) fn registry() -> Vec<Shortcut> {
    vec![
        primary_shortcut!(
            "Sessions",
            "Open first unsubmitted draft",
            "0",
            SwitchSession0
        ),
        primary_shortcut!("Sessions", "Open session 1", "1", SwitchSession1),
        primary_shortcut!("Sessions", "Open session 2", "2", SwitchSession2),
        primary_shortcut!("Sessions", "Open session 3", "3", SwitchSession3),
        primary_shortcut!("Sessions", "Open session 4", "4", SwitchSession4),
        primary_shortcut!("Sessions", "Open session 5", "5", SwitchSession5),
        primary_shortcut!("Sessions", "Open session 6", "6", SwitchSession6),
        primary_shortcut!("Sessions", "Open session 7", "7", SwitchSession7),
        primary_shortcut!("Sessions", "Open session 8", "8", SwitchSession8),
        primary_shortcut!("Sessions", "Open session 9", "9", SwitchSession9),
        primary_shortcut!("Sessions", "New session", "n", NewSession),
        primary_shortcut!("Sessions", "Add project", "shift-n", AddProject),
        primary_shortcut!("Application", "Open action picker", "k", ShowActionPicker),
        primary_shortcut!("Sessions", "Previous session", "[", PreviousSession),
        primary_shortcut!("Sessions", "Next session", "]", NextSession),
        primary_shortcut!(
            "Sessions",
            "Show archived sessions",
            "shift-a",
            ToggleArchivedSessions
        ),
        primary_shortcut!(
            "Sessions",
            "Close workspace, draft, or session",
            "w",
            CloseCurrent
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
        Shortcut {
            section: "Composer",
            label: "Previous completion",
            keystroke: "ctrl-p",
            show_in_help: false,
            binding: KeyBinding::new(
                "ctrl-p",
                ComposerHistoryPrevious,
                Some("PiComposer > Input"),
            ),
        },
        Shortcut {
            section: "Composer",
            label: "Next completion",
            keystroke: "ctrl-n",
            show_in_help: false,
            binding: KeyBinding::new("ctrl-n", ComposerHistoryNext, Some("PiComposer > Input")),
        },
        primary_shortcut!("Workspace", "Chat and composer", "l", FocusComposer),
        primary_shortcut!("Workspace", "Open Neovim", "e", ShowEditor),
        primary_shortcut!("Workspace", "Open terminal", "t", ShowTerminal),
        Shortcut {
            section: "Transcript",
            label: "Copy visual selection",
            keystroke: primary!("c"),
            show_in_help: false,
            binding: KeyBinding::new(
                primary!("c"),
                CopySelection,
                Some(TRANSCRIPT_SELECTION_KEY_CONTEXT),
            ),
        },
        Shortcut {
            section: "Composer",
            label: "Copy selection",
            keystroke: primary!("c"),
            show_in_help: false,
            binding: KeyBinding::new(primary!("c"), CopySelection, Some("PiComposer > Input")),
        },
        primary_shortcut!("Composer", "Send prompt", "enter", SubmitPrompt),
        shortcut!(
            "Composer",
            "Send follow-up",
            "tab",
            SubmitFollowUp,
            Some("PiComposer > Input")
        ),
        primary_shortcut!("Run", "Abort current run", ".", AbortRun),
        shortcut!(
            "Run",
            "Apply queued steer, double-Esc aborts",
            "escape",
            ComposerEscape,
            Some("PiComposer > Input")
        ),
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
        primary_shortcut!(
            "Application",
            "Open / close project work",
            "shift-i",
            ShowWorkGraph
        ),
        primary_shortcut!("Application", "Keyboard shortcuts", "?", ShowKeybindings),
        Shortcut {
            section: "Application",
            label: "Keyboard shortcuts",
            keystroke: primary!("/"),
            show_in_help: false,
            binding: KeyBinding::new(primary!("/"), ShowKeybindings, None),
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
        primary_shortcut!("Application", "Quit", "q", QuitApplication),
    ]
}

#[cfg(test)]
mod tests {
    use super::{bindings, primary_key, registry};

    #[test]
    fn root_focus_traversal_is_unbound() {
        let bindings = bindings();
        let root_context = gpui::KeyBindingContextPredicate::parse("Root").expect("root context");
        for (keystroke, target) in [("tab", "root::Tab"), ("shift-tab", "root::TabPrev")] {
            assert!(bindings.iter().any(|binding| {
                binding
                    .action()
                    .as_any()
                    .downcast_ref::<gpui::Unbind>()
                    .is_some_and(|unbind| unbind.0.as_ref() == target)
                    && binding.match_keystrokes(&[
                        gpui::Keystroke::parse(keystroke).expect("test keystroke")
                    ]) == Some(false)
                    && binding.predicate().as_deref() == Some(&root_context)
            }));
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_application_shortcuts_do_not_use_the_system_modifier() {
        let shortcuts = registry();
        assert!(
            shortcuts
                .iter()
                .all(|shortcut| !shortcut.keystroke.starts_with("cmd-"))
        );
        assert!(
            shortcuts.iter().any(|shortcut| {
                shortcut.label == "New session" && shortcut.keystroke == "ctrl-n"
            })
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_application_shortcuts_keep_the_command_modifier() {
        let shortcuts = registry();
        assert!(
            shortcuts.iter().any(|shortcut| {
                shortcut.label == "New session" && shortcut.keystroke == "cmd-n"
            })
        );
    }

    #[test]
    fn copy_shortcuts_route_through_the_application_command() {
        let shortcuts = registry();
        assert!(shortcuts.iter().any(|shortcut| {
            shortcut.label == "Copy visual selection" && shortcut.keystroke == primary!("c")
        }));
        assert!(shortcuts.iter().any(|shortcut| {
            shortcut.label == "Copy selection" && shortcut.keystroke == primary!("c")
        }));
    }

    #[test]
    fn workspace_shortcuts_are_registered() {
        let shortcuts = registry();
        for (label, keystroke) in [
            ("Chat and composer", primary!("l")),
            ("Open Neovim", primary!("e")),
            ("Open terminal", primary!("t")),
        ] {
            assert!(
                shortcuts
                    .iter()
                    .any(|shortcut| { shortcut.label == label && shortcut.keystroke == keystroke })
            );
        }
    }

    #[test]
    fn keyboard_help_has_both_question_mark_shortcuts_and_workgraph_navigation() {
        let shortcuts = registry();
        assert!(shortcuts.iter().any(|shortcut| {
            shortcut.label == "Open first unsubmitted draft" && shortcut.keystroke == primary!("0")
        }));
        assert!(
            shortcuts
                .iter()
                .any(|shortcut| shortcut.keystroke == primary!("?"))
        );
        assert!(
            shortcuts
                .iter()
                .any(|shortcut| shortcut.keystroke == primary!("/"))
        );
        assert!(shortcuts.iter().any(|shortcut| {
            shortcut.label == "Open / close project work"
                && shortcut.keystroke == primary!("shift-i")
        }));
        assert!(shortcuts.iter().any(|shortcut| {
            shortcut.label == "Close workspace, draft, or session"
                && shortcut.keystroke == primary!("w")
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
        assert!(shortcuts.iter().any(|shortcut| {
            shortcut.label == "Previous completion" && shortcut.keystroke == "ctrl-p"
        }));
        assert!(shortcuts.iter().any(|shortcut| {
            shortcut.label == "Next completion" && shortcut.keystroke == "ctrl-n"
        }));
    }
}
