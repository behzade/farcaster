//! Single registry for application keybindings and keyboard-help metadata.

#[cfg(not(target_os = "macos"))]
use crate::app::APP_SHORTCUT_CONTEXT;
#[cfg(target_os = "macos")]
use crate::app::ShowActionPicker;
use crate::app::{
    AbortRun, AddProject, CloseCurrent, ComposerCompletionNext, ComposerCompletionPrevious,
    ComposerEscape, ComposerHistoryNext, ComposerHistoryPrevious, DismissSurface, FocusComposer,
    NewSession, NextSession, OVERLAY_KEY_CONTEXT, PICKER_KEY_CONTEXT, PickerBack, PreviousSession,
    QuitApplication, ShowEditor, ShowKeybindings, ShowTerminal, ShowWorkGraph, SubmitFollowUp,
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
#[cfg(not(target_os = "macos"))]
use gpui_component::input::{Copy as InputCopy, Paste};

#[cfg(target_os = "macos")]
pub(crate) const PRIMARY_MODIFIER: &str = "cmd";
#[cfg(not(target_os = "macos"))]
pub(crate) const PRIMARY_MODIFIER: &str = "ctrl";

#[cfg(target_os = "macos")]
const PRIMARY_SHORTCUT_CONTEXT: Option<&str> = None;
#[cfg(not(target_os = "macos"))]
const PRIMARY_SHORTCUT_CONTEXT: Option<&str> = Some(APP_SHORTCUT_CONTEXT);

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
        shortcut!(
            $section,
            $label,
            primary!($key),
            $action,
            PRIMARY_SHORTCUT_CONTEXT
        )
    };
}

#[cfg(not(target_os = "macos"))]
macro_rules! linux_super_binding {
    ($key:literal, $action:expr) => {
        linux_super_binding!($key, $action, Some(APP_SHORTCUT_CONTEXT))
    };
    ($key:literal, $action:expr, $context:expr) => {
        KeyBinding::new(concat!("super-", $key), $action, $context)
    };
}

#[cfg(target_os = "macos")]
fn platform_shortcut_aliases() -> [Shortcut; 4] {
    [
        Shortcut {
            section: "Workspace",
            label: "Chat and composer",
            keystroke: "cmd-l",
            show_in_help: false,
            binding: KeyBinding::new("cmd-l", FocusComposer, None),
        },
        Shortcut {
            section: "Workspace",
            label: "Open Neovim",
            keystroke: "cmd-e",
            show_in_help: false,
            binding: KeyBinding::new("cmd-e", ShowEditor, None),
        },
        Shortcut {
            section: "Workspace",
            label: "Open terminal",
            keystroke: "cmd-t",
            show_in_help: false,
            binding: KeyBinding::new("cmd-t", ShowTerminal, None),
        },
        Shortcut {
            section: "Application",
            label: "Open action picker",
            keystroke: "cmd-k",
            show_in_help: true,
            binding: KeyBinding::new("cmd-k", ShowActionPicker, None),
        },
    ]
}

#[cfg(not(target_os = "macos"))]
const fn platform_shortcut_aliases() -> [Shortcut; 0] {
    []
}

#[cfg(not(target_os = "macos"))]
fn linux_super_bindings() -> Vec<KeyBinding> {
    vec![
        linux_super_binding!("0", SwitchSession0),
        linux_super_binding!("1", SwitchSession1),
        linux_super_binding!("2", SwitchSession2),
        linux_super_binding!("3", SwitchSession3),
        linux_super_binding!("4", SwitchSession4),
        linux_super_binding!("5", SwitchSession5),
        linux_super_binding!("6", SwitchSession6),
        linux_super_binding!("7", SwitchSession7),
        linux_super_binding!("8", SwitchSession8),
        linux_super_binding!("9", SwitchSession9),
        linux_super_binding!("t", NewSession),
        linux_super_binding!("shift-n", AddProject),
        linux_super_binding!("[", PreviousSession),
        linux_super_binding!("]", NextSession),
        linux_super_binding!("shift-a", ToggleArchivedSessions),
        linux_super_binding!("w", CloseCurrent),
        linux_super_binding!("p", ComposerCompletionPrevious, Some("PiComposer > Input")),
        linux_super_binding!("n", ComposerCompletionNext, Some("PiComposer > Input")),
        linux_super_binding!("c", CopySelection, Some(TRANSCRIPT_SELECTION_KEY_CONTEXT)),
        linux_super_binding!("c", CopySelection, Some("PiComposer > Input")),
        linux_super_binding!("c", InputCopy, Some("Input")),
        linux_super_binding!("v", Paste, Some("Input")),
        linux_super_binding!("enter", SubmitPrompt),
        linux_super_binding!(".", AbortRun),
        linux_super_binding!("shift-i", ShowWorkGraph),
        linux_super_binding!("?", ShowKeybindings),
        linux_super_binding!("/", ShowKeybindings),
        linux_super_binding!("p", SelectUp, Some("PiPicker > Input")),
        linux_super_binding!("n", SelectDown, Some("PiPicker > Input")),
        linux_super_binding!("q", QuitApplication),
    ]
}

/// Returns application shortcuts plus overrides for dependency-owned defaults.
pub(crate) fn bindings() -> Vec<KeyBinding> {
    let mut bindings = registry()
        .into_iter()
        .map(|shortcut| shortcut.binding)
        .collect::<Vec<_>>();
    #[cfg(not(target_os = "macos"))]
    bindings.extend(linux_super_bindings());
    bindings.extend([
        KeyBinding::new("tab", Unbind("root::Tab".into()), Some("Root")),
        KeyBinding::new("shift-tab", Unbind("root::TabPrev".into()), Some("Root")),
    ]);
    bindings
}

/// Returns the user-facing shortcut registry used by keyboard help.
pub(crate) fn registry() -> Vec<Shortcut> {
    let mut shortcuts = vec![
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
        shortcut!(
            "Sessions",
            "New session",
            primary_key("cmd-n", "ctrl-t"),
            NewSession,
            PRIMARY_SHORTCUT_CONTEXT
        ),
        primary_shortcut!("Sessions", "Add project", "shift-n", AddProject),
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
                ComposerCompletionPrevious,
                Some("PiComposer > Input"),
            ),
        },
        Shortcut {
            section: "Composer",
            label: "Next completion",
            keystroke: "ctrl-n",
            show_in_help: false,
            binding: KeyBinding::new("ctrl-n", ComposerCompletionNext, Some("PiComposer > Input")),
        },
        shortcut!("Workspace", "Chat and composer", "f1", FocusComposer, None),
        shortcut!("Workspace", "Open Neovim", "f2", ShowEditor, None),
        shortcut!("Workspace", "Open terminal", "f3", ShowTerminal, None),
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
            binding: KeyBinding::new(primary!("/"), ShowKeybindings, PRIMARY_SHORTCUT_CONTEXT),
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
    ];

    shortcuts.extend(platform_shortcut_aliases());
    shortcuts
}

#[cfg(test)]
mod tests {
    use super::{bindings, primary_key, registry};

    #[cfg(not(target_os = "macos"))]
    use crate::app::{APP_INPUT_CONTEXT, ComposerCompletionNext, NATIVE_INPUT_CONTEXT};
    #[cfg(not(target_os = "macos"))]
    use gpui_component::input::Paste;

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
    fn non_macos_shortcuts_avoid_native_input_conflicts() {
        let shortcuts = registry();
        let bindings = bindings();
        assert!(
            shortcuts
                .iter()
                .all(|shortcut| !shortcut.keystroke.starts_with("cmd-"))
        );
        for removed in ["ctrl-l", "ctrl-e", "ctrl-k"] {
            assert!(
                shortcuts
                    .iter()
                    .all(|shortcut| shortcut.keystroke != removed)
            );
        }
        for shortcut in shortcuts
            .iter()
            .filter(|shortcut| shortcut.keystroke.starts_with("ctrl-"))
        {
            let super_keystroke = shortcut.keystroke.replacen("ctrl-", "super-", 1);
            assert!(bindings.iter().any(|binding| {
                binding.keystrokes().len() == 1
                    && binding.keystrokes()[0].unparse() == super_keystroke
                    && binding.action().name() == shortcut.binding.action().name()
            }));
        }

        let keymap = gpui::Keymap::new(bindings);
        let app_context = gpui::KeyContext::parse(APP_INPUT_CONTEXT).expect("app context");
        let native_context = gpui::KeyContext::parse(NATIVE_INPUT_CONTEXT).expect("native context");
        for keystroke in ["ctrl-t", "super-t"] {
            let keystroke = gpui::Keystroke::parse(keystroke).expect("shortcut keystroke");
            assert!(
                !keymap
                    .bindings_for_input(&[keystroke.clone()], &[app_context.clone()])
                    .0
                    .is_empty()
            );
            assert!(
                keymap
                    .bindings_for_input(&[keystroke], &[native_context.clone()])
                    .0
                    .is_empty()
            );
        }
        let input_context = gpui::KeyContext::parse("Input").expect("input context");
        let (paste, _) = keymap.bindings_for_input(
            &[gpui::Keystroke::parse("super-v").expect("paste keystroke")],
            &[app_context, input_context],
        );
        assert!(
            paste
                .first()
                .is_some_and(|binding| { binding.action().as_any().is::<Paste>() })
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn composer_ctrl_n_remains_completion_navigation() {
        let keymap = gpui::Keymap::new(
            registry()
                .into_iter()
                .map(|shortcut| shortcut.binding)
                .collect(),
        );
        let contexts = [
            gpui::KeyContext::parse(APP_INPUT_CONTEXT).expect("app context"),
            gpui::KeyContext::parse("PiComposer").expect("composer context"),
            gpui::KeyContext::parse("Input").expect("input context"),
        ];
        let (bindings, _) = keymap.bindings_for_input(
            &[gpui::Keystroke::parse("ctrl-n").expect("shortcut keystroke")],
            &contexts,
        );

        assert!(
            bindings.first().is_some_and(|binding| {
                binding.action().as_any().is::<ComposerCompletionNext>()
            })
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_application_shortcuts_keep_the_global_command_modifier() {
        let shortcuts = registry();
        for (label, keystroke) in [
            ("New session", "cmd-n"),
            ("Chat and composer", "cmd-l"),
            ("Open Neovim", "cmd-e"),
            ("Open terminal", "cmd-t"),
            ("Open action picker", "cmd-k"),
        ] {
            assert!(shortcuts.iter().any(|shortcut| {
                shortcut.label == label
                    && shortcut.keystroke == keystroke
                    && shortcut.binding.predicate().is_none()
            }));
        }
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
    }
}
