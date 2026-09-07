use crate::app::ui::keyboard::CopySelection;
use crate::app::workspace::{CycleWorkspaceBackward, CycleWorkspaceForward};
use crate::app::{APP_SHORTCUT_CONTEXT, TRANSCRIPT_SELECTION_KEY_CONTEXT};
use crate::app::{
    AbortRun, AddProject, CloseCurrent, ComposerCompletionNext, ComposerCompletionPrevious,
    ComposerEscape, ComposerHistoryNext, ComposerHistoryPrevious, DismissSurface, FocusComposer,
    NewSession, NextSession, OVERLAY_KEY_CONTEXT, PICKER_KEY_CONTEXT, PickerBack, PreviousSession,
    QuitApplication, ShowActionPicker, ShowEditor, ShowKeybindings, ShowTerminal, ShowWorkGraph,
    SubmitFollowUp, SwitchSession0, SwitchSession1, SwitchSession2, SwitchSession3, SwitchSession4,
    SwitchSession5, SwitchSession6, SwitchSession7, SwitchSession8, SwitchSession9,
    WorkCreateIssue, WorkDismiss, WorkFocusSearch, WorkNextIssue, WorkPreviousIssue,
};
use crate::app::{WORKGRAPH_KEY_CONTEXT, WORKGRAPH_NAV_KEY_CONTEXT};
use gpui::{KeyBinding, Unbind};
use gpui_base::actions::{SelectDown, SelectUp};

pub(crate) fn application_key(suffix: &str) -> String {
    format!("{}-{suffix}", platform_key("cmd", "ctrl"))
}

const fn platform_key(macos: &'static str, non_macos: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        macos
    } else {
        non_macos
    }
}

pub(crate) struct Shortcut {
    pub section: &'static str,
    pub label: &'static str,
    pub keystroke: String,
    pub show_in_help: bool,
    pub binding: KeyBinding,
}

macro_rules! platform {
    ($key:literal) => {
        platform_key(concat!("cmd-", $key), concat!("ctrl-", $key))
    };
}

macro_rules! shortcut {
    ($section:literal, $label:literal, $key:expr, $action:expr, $context:expr) => {
        shortcut!($section, $label, $key, $action, $context, true)
    };
    ($section:literal, $label:literal, $key:expr, $action:expr, $context:expr, $show:expr) => {{
        let key = $key.to_string();
        Shortcut {
            section: $section,
            label: $label,
            keystroke: key.clone(),
            show_in_help: $show,
            binding: KeyBinding::new(&key, $action, $context),
        }
    }};
}

pub(crate) fn bindings() -> Vec<KeyBinding> {
    let mut bindings = registry()
        .into_iter()
        .map(|shortcut| shortcut.binding)
        .collect::<Vec<_>>();
    bindings.extend([
        KeyBinding::new("tab", Unbind("root::Tab".into()), Some("Root")),
        KeyBinding::new("shift-tab", Unbind("root::TabPrev".into()), Some("Root")),
    ]);
    bindings
}

pub(crate) fn registry() -> Vec<Shortcut> {
    registry_for_platform(platform_key("cmd", "ctrl"))
}

fn registry_for_platform(prefix: &str) -> Vec<Shortcut> {
    macro_rules! application_shortcut {
        ($section:literal, $label:literal, $key:literal, $action:expr) => {
            application_shortcut!($section, $label, $key, $action, true)
        };
        ($section:literal, $label:literal, $key:literal, $action:expr, $show:expr) => {
            shortcut!(
                $section,
                $label,
                format!("{prefix}-{}", $key),
                $action,
                Some(APP_SHORTCUT_CONTEXT),
                $show
            )
        };
    }
    let mut shortcuts = vec![
        application_shortcut!(
            "Sessions",
            "Open first unsubmitted draft",
            "0",
            SwitchSession0
        ),
        application_shortcut!("Sessions", "Open session 1", "1", SwitchSession1),
        application_shortcut!("Sessions", "Open session 2", "2", SwitchSession2),
        application_shortcut!("Sessions", "Open session 3", "3", SwitchSession3),
        application_shortcut!("Sessions", "Open session 4", "4", SwitchSession4),
        application_shortcut!("Sessions", "Open session 5", "5", SwitchSession5),
        application_shortcut!("Sessions", "Open session 6", "6", SwitchSession6),
        application_shortcut!("Sessions", "Open session 7", "7", SwitchSession7),
        application_shortcut!("Sessions", "Open session 8", "8", SwitchSession8),
        application_shortcut!("Sessions", "Open session 9", "9", SwitchSession9),
        application_shortcut!("Sessions", "New session", "t", NewSession),
        application_shortcut!("Sessions", "Add project", "shift-n", AddProject),
        application_shortcut!(
            "Configuration",
            "Set sandbox",
            "shift-s",
            crate::app::SetSandbox
        ),
        application_shortcut!(
            "Configuration",
            "Set provider/model/effort",
            "shift-m",
            crate::app::SetRuntime
        ),
        application_shortcut!("Sessions", "Previous session", "[", PreviousSession),
        application_shortcut!("Sessions", "Next session", "]", NextSession),
        application_shortcut!(
            "Sessions",
            "Restore session",
            "shift-a",
            crate::app::RestoreSession
        ),
        application_shortcut!(
            "Sessions",
            "Close surface or draft; archive session",
            "w",
            CloseCurrent
        ),
        Shortcut {
            section: "Composer",
            label: "Previous prompt",
            keystroke: "up".into(),
            show_in_help: false,
            binding: KeyBinding::new(
                "up",
                ComposerHistoryPrevious,
                Some("FarcasterComposer > Input"),
            ),
        },
        Shortcut {
            section: "Composer",
            label: "Next prompt",
            keystroke: "down".into(),
            show_in_help: false,
            binding: KeyBinding::new(
                "down",
                ComposerHistoryNext,
                Some("FarcasterComposer > Input"),
            ),
        },
        Shortcut {
            section: "Composer",
            label: "Previous completion",
            keystroke: "ctrl-p".into(),
            show_in_help: false,
            binding: KeyBinding::new(
                "ctrl-p",
                ComposerCompletionPrevious,
                Some("FarcasterComposer > Input"),
            ),
        },
        Shortcut {
            section: "Composer",
            label: "Next completion",
            keystroke: "ctrl-n".into(),
            show_in_help: false,
            binding: KeyBinding::new(
                "ctrl-n",
                ComposerCompletionNext,
                Some("FarcasterComposer > Input"),
            ),
        },
        application_shortcut!("Workspace", "Chat and composer", "l", FocusComposer),
        #[cfg(target_os = "macos")]
        shortcut!(
            "Workspace",
            "Chat composer",
            "cmd-g",
            FocusComposer,
            Some(APP_SHORTCUT_CONTEXT)
        ),
        shortcut!(
            "Workspace",
            "Chat and composer",
            "f1",
            FocusComposer,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
        application_shortcut!("Workspace", "Open Neovim", "e", ShowEditor),
        shortcut!(
            "Workspace",
            "Open Neovim",
            "f2",
            ShowEditor,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
        application_shortcut!("Workspace", "Open terminal", "j", ShowTerminal),
        shortcut!(
            "Workspace",
            "Open terminal",
            "f3",
            ShowTerminal,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
        shortcut!(
            "Workspace",
            "Next workspace surface",
            "ctrl-tab",
            CycleWorkspaceForward,
            Some(APP_SHORTCUT_CONTEXT)
        ),
        shortcut!(
            "Workspace",
            "Previous workspace surface",
            "ctrl-shift-tab",
            CycleWorkspaceBackward,
            Some(APP_SHORTCUT_CONTEXT)
        ),
        Shortcut {
            section: "Transcript",
            label: "Copy transcript selection",
            keystroke: platform!("c").into(),
            show_in_help: false,
            binding: KeyBinding::new(
                platform!("c"),
                CopySelection,
                Some(TRANSCRIPT_SELECTION_KEY_CONTEXT),
            ),
        },
        Shortcut {
            section: "Composer",
            label: "Copy selection",
            keystroke: platform!("c").into(),
            show_in_help: false,
            binding: KeyBinding::new(
                platform!("c"),
                CopySelection,
                Some("FarcasterComposer > Input"),
            ),
        },
        shortcut!(
            "Composer",
            "Queue follow-up",
            "tab",
            SubmitFollowUp,
            Some("FarcasterComposer > Input")
        ),
        application_shortcut!("Run", "Abort current run", ".", AbortRun),
        shortcut!(
            "Run",
            "Normal when idle; apply steer, double-Esc aborts",
            "escape",
            ComposerEscape,
            Some("FarcasterComposer > Input")
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
        application_shortcut!(
            "Application",
            "Open / close project work",
            "shift-i",
            ShowWorkGraph
        ),
        application_shortcut!("Application", "Open action picker", "k", ShowActionPicker),
        shortcut!(
            "Application",
            "Open action picker",
            "f4",
            ShowActionPicker,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
        application_shortcut!("Application", "Keyboard shortcuts", "/", ShowKeybindings),
        application_shortcut!(
            "Application",
            "Keyboard shortcuts",
            "?",
            ShowKeybindings,
            false
        ),
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
            keystroke: "ctrl-p".into(),
            show_in_help: false,
            binding: KeyBinding::new("ctrl-p", SelectUp, Some("PiPicker > Input")),
        },
        Shortcut {
            section: "Application",
            label: "Next picker item",
            keystroke: "ctrl-n".into(),
            show_in_help: false,
            binding: KeyBinding::new("ctrl-n", SelectDown, Some("PiPicker > Input")),
        },
        Shortcut {
            section: "Application",
            label: "Back in action picker",
            keystroke: "backspace".into(),
            show_in_help: false,
            binding: KeyBinding::new("backspace", PickerBack, Some("PiPicker > Input")),
        },
        Shortcut {
            section: "Application",
            label: "Close action picker",
            keystroke: "escape".into(),
            show_in_help: false,
            binding: KeyBinding::new("escape", DismissSurface, Some(PICKER_KEY_CONTEXT)),
        },
        application_shortcut!("Application", "Quit", "q", QuitApplication),
        #[cfg(not(target_os = "macos"))]
        shortcut!(
            "Sessions",
            "New session",
            "ctrl-t",
            NewSession,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
        #[cfg(not(target_os = "macos"))]
        shortcut!(
            "Sessions",
            "Close surface or draft; archive session",
            "ctrl-w",
            CloseCurrent,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
        #[cfg(not(target_os = "macos"))]
        shortcut!(
            "Sessions",
            "Open first unsubmitted draft",
            "ctrl-0",
            SwitchSession0,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
        #[cfg(not(target_os = "macos"))]
        shortcut!(
            "Sessions",
            "Open session 1",
            "ctrl-1",
            SwitchSession1,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
        #[cfg(not(target_os = "macos"))]
        shortcut!(
            "Sessions",
            "Open session 2",
            "ctrl-2",
            SwitchSession2,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
        #[cfg(not(target_os = "macos"))]
        shortcut!(
            "Sessions",
            "Open session 3",
            "ctrl-3",
            SwitchSession3,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
        #[cfg(not(target_os = "macos"))]
        shortcut!(
            "Sessions",
            "Open session 4",
            "ctrl-4",
            SwitchSession4,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
        #[cfg(not(target_os = "macos"))]
        shortcut!(
            "Sessions",
            "Open session 5",
            "ctrl-5",
            SwitchSession5,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
        #[cfg(not(target_os = "macos"))]
        shortcut!(
            "Sessions",
            "Open session 6",
            "ctrl-6",
            SwitchSession6,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
        #[cfg(not(target_os = "macos"))]
        shortcut!(
            "Sessions",
            "Open session 7",
            "ctrl-7",
            SwitchSession7,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
        #[cfg(not(target_os = "macos"))]
        shortcut!(
            "Sessions",
            "Open session 8",
            "ctrl-8",
            SwitchSession8,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
        #[cfg(not(target_os = "macos"))]
        shortcut!(
            "Sessions",
            "Open session 9",
            "ctrl-9",
            SwitchSession9,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
    ];
    if prefix == "ctrl" {
        shortcuts.retain(|shortcut| !matches!(shortcut.keystroke.as_str(), "ctrl-j" | "ctrl-k"));
    }
    shortcuts
}

#[cfg(test)]
mod tests {
    use super::{application_key, bindings, platform_key, registry};

    use crate::app::ComposerCompletionNext;

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

    #[test]
    fn configuration_shortcuts_route_to_actions_only_in_app_views() {
        use super::registry_for_platform;
        use crate::app::{
            APP_INPUT_CONTEXT, NATIVE_INPUT_CONTEXT, RestoreSession, SetRuntime, SetSandbox,
        };

        for prefix in ["cmd", "ctrl"] {
            let keymap = gpui::Keymap::new(
                registry_for_platform(prefix)
                    .into_iter()
                    .map(|shortcut| shortcut.binding)
                    .collect(),
            );
            for (suffix, action) in [
                ("shift-s", Box::new(SetSandbox) as Box<dyn gpui::Action>),
                ("shift-m", Box::new(SetRuntime) as Box<dyn gpui::Action>),
                ("shift-a", Box::new(RestoreSession) as Box<dyn gpui::Action>),
            ] {
                let stroke = gpui::Keystroke::parse(&format!("{prefix}-{suffix}")).unwrap();
                let (bindings, _) = keymap.bindings_for_input(
                    &[stroke.clone()],
                    &[gpui::KeyContext::parse(APP_INPUT_CONTEXT).unwrap()],
                );
                assert_eq!(bindings.first().unwrap().action().name(), action.name());
                let (bindings, _) = keymap.bindings_for_input(
                    &[stroke],
                    &[gpui::KeyContext::parse(NATIVE_INPUT_CONTEXT).unwrap()],
                );
                assert!(bindings.is_empty());
            }
            let (bindings, _) = keymap.bindings_for_input(
                &[gpui::Keystroke::parse(&format!("{prefix}-m")).unwrap()],
                &[gpui::KeyContext::parse(APP_INPUT_CONTEXT).unwrap()],
            );
            assert!(bindings.is_empty(), "unshifted M must remain unbound");
        }
    }

    #[test]
    fn composer_ctrl_n_remains_completion_navigation() {
        let keymap = gpui::Keymap::new(
            registry()
                .into_iter()
                .map(|shortcut| shortcut.binding)
                .collect(),
        );
        let contexts = [
            gpui::KeyContext::parse("FarcasterComposer").expect("composer context"),
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

    #[test]
    fn legacy_global_actions_are_scoped_to_app_owned_contexts() {
        use crate::app::APP_SHORTCUT_CONTEXT;
        let app_context = gpui::KeyBindingContextPredicate::parse(APP_SHORTCUT_CONTEXT)
            .expect("app shortcut context");
        let shortcuts = registry();
        for keystroke in ["f1", "f2", "f3", "f4", "ctrl-tab", "ctrl-shift-tab"] {
            let matches = shortcuts
                .iter()
                .filter(|shortcut| shortcut.keystroke == keystroke)
                .collect::<Vec<_>>();
            assert!(!matches.is_empty(), "{keystroke} must remain registered");
            for shortcut in matches {
                assert_eq!(
                    shortcut.binding.predicate().as_deref(),
                    Some(&app_context),
                    "{keystroke} must not be a global None-context binding"
                );
            }
        }
    }

    #[test]
    fn application_shortcuts_stay_in_app_owned_contexts() {
        use super::{APP_SHORTCUT_CONTEXT, registry_for_platform};
        use crate::app::{APP_INPUT_CONTEXT, NATIVE_INPUT_CONTEXT};
        let app_context = gpui::KeyBindingContextPredicate::parse(APP_SHORTCUT_CONTEXT)
            .expect("app shortcut context");
        let shortcuts = registry();
        for (label, suffix) in [
            ("New session", "t"),
            ("Chat and composer", "l"),
            ("Open Neovim", "e"),
            ("Open terminal", "j"),
            ("Open action picker", "k"),
        ] {
            let keystroke = application_key(suffix);
            if matches!(keystroke.as_str(), "ctrl-j" | "ctrl-k") {
                continue;
            }
            assert!(shortcuts.iter().any(|shortcut| {
                shortcut.label == label
                    && shortcut.keystroke == keystroke
                    && shortcut.binding.predicate().as_deref() == Some(&app_context)
            }));
        }

        let keymap = gpui::Keymap::new(
            registry_for_platform("cmd")
                .into_iter()
                .map(|shortcut| shortcut.binding)
                .collect(),
        );
        let app_contexts = [gpui::KeyContext::parse(APP_INPUT_CONTEXT).unwrap()];
        let native_contexts = [gpui::KeyContext::parse(NATIVE_INPUT_CONTEXT).unwrap()];
        for key in ["cmd-2", "cmd-t", "cmd-e", "cmd-j", "cmd-g"] {
            if key == "cmd-g" && !cfg!(target_os = "macos") {
                continue;
            }
            let stroke = gpui::Keystroke::parse(key).unwrap();
            let (app_bindings, _) = keymap.bindings_for_input(&[stroke.clone()], &app_contexts);
            assert!(!app_bindings.is_empty(), "{key} missing in app context");
            let (native_bindings, _) = keymap.bindings_for_input(&[stroke], &native_contexts);
            assert!(
                native_bindings.is_empty(),
                "{key} must not reach embedded views"
            );
        }
        let (ctrl_j, _) =
            keymap.bindings_for_input(&[gpui::Keystroke::parse("ctrl-j").unwrap()], &app_contexts);
        assert!(ctrl_j.is_empty(), "Ctrl+J must not open the terminal");
        for key in ["f1", "f2", "f3", "f4", "ctrl-tab", "ctrl-shift-tab"] {
            let stroke = gpui::Keystroke::parse(key).unwrap();
            let (native_bindings, _) =
                keymap.bindings_for_input(&[stroke.clone()], &native_contexts);
            assert!(
                native_bindings.is_empty(),
                "{key} must not reach embedded views"
            );
            let (app_bindings, _) = keymap.bindings_for_input(&[stroke], &app_contexts);
            assert!(!app_bindings.is_empty(), "{key} missing in app context");
        }

        let control_map = gpui::Keymap::new(
            registry_for_platform("ctrl")
                .into_iter()
                .map(|shortcut| shortcut.binding)
                .collect(),
        );
        for key in ["ctrl-j", "ctrl-k"] {
            let (bindings, _) = control_map
                .bindings_for_input(&[gpui::Keystroke::parse(key).unwrap()], &app_contexts);
            assert!(
                bindings.is_empty(),
                "{key} must stay available to chat input when Control is the modifier"
            );
        }
    }

    #[test]
    fn copy_shortcuts_route_through_the_application_command() {
        let shortcuts = registry();
        assert!(shortcuts.iter().any(|shortcut| {
            shortcut.label == "Copy transcript selection" && shortcut.keystroke == platform!("c")
        }));
        assert!(shortcuts.iter().any(|shortcut| {
            shortcut.label == "Copy selection" && shortcut.keystroke == platform!("c")
        }));
    }
}
