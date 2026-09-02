use std::sync::OnceLock;

use crate::app::ui::keyboard::CopySelection;
use crate::app::{APP_SHORTCUT_CONTEXT, TRANSCRIPT_SELECTION_KEY_CONTEXT};
use crate::app::{
    AbortRun, AddProject, CloseCurrent, ComposerCompletionNext, ComposerCompletionPrevious,
    ComposerEscape, ComposerHistoryNext, ComposerHistoryPrevious, DismissSurface, FocusComposer,
    NewSession, NextSession, OVERLAY_KEY_CONTEXT, PICKER_KEY_CONTEXT, PickerBack, PreviousSession,
    QuitApplication, ShowActionPicker, ShowEditor, ShowKeybindings, ShowTerminal, ShowWorkGraph,
    SubmitFollowUp, SwitchSession0, SwitchSession1, SwitchSession2, SwitchSession3, SwitchSession4,
    SwitchSession5, SwitchSession6, SwitchSession7, SwitchSession8, SwitchSession9,
    ToggleArchivedSessions, WorkCreateIssue, WorkDismiss, WorkFocusSearch, WorkNextIssue,
    WorkPreviousIssue,
};
use crate::app::{WORKGRAPH_KEY_CONTEXT, WORKGRAPH_NAV_KEY_CONTEXT};
use gpui::{KeyBinding, Unbind};
use gpui_base::actions::{SelectDown, SelectUp};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ApplicationModifier {
    Command,
    Super,
    Control,
    Alt,
}

impl ApplicationModifier {
    pub(crate) const fn prefix(self) -> &'static str {
        match self {
            Self::Command => "cmd",
            Self::Super => "super",
            Self::Control => "ctrl",
            Self::Alt => "alt",
        }
    }

    fn key(self, suffix: &str) -> String {
        format!("{}-{suffix}", self.prefix())
    }
}

fn parse_application_modifier(value: Option<&str>) -> ApplicationModifier {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("cmd" | "command") => ApplicationModifier::Command,
        Some("super" | "meta") => ApplicationModifier::Super,
        Some("ctrl" | "control") => ApplicationModifier::Control,
        Some("alt" | "option") => ApplicationModifier::Alt,
        _ if cfg!(target_os = "macos") => ApplicationModifier::Command,
        _ => ApplicationModifier::Super,
    }
}

/// Returns the process-wide application modifier. Override the platform default
/// with `FARCASTER_APP_MODIFIER=cmd|super|ctrl|alt` before launch.
pub(crate) fn application_modifier() -> ApplicationModifier {
    static MODIFIER: OnceLock<ApplicationModifier> = OnceLock::new();
    *MODIFIER.get_or_init(|| {
        parse_application_modifier(std::env::var("FARCASTER_APP_MODIFIER").ok().as_deref())
    })
}

pub(crate) fn application_key(suffix: &str) -> String {
    application_modifier().key(suffix)
}

fn application_context() -> Option<&'static str> {
    match application_modifier() {
        ApplicationModifier::Command | ApplicationModifier::Super => None,
        ApplicationModifier::Control | ApplicationModifier::Alt => Some(APP_SHORTCUT_CONTEXT),
    }
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

macro_rules! application_shortcut {
    ($section:literal, $label:literal, $key:literal, $action:expr) => {
        shortcut!(
            $section,
            $label,
            application_key($key),
            $action,
            application_context()
        )
    };
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
    vec![
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
        application_shortcut!("Sessions", "Previous session", "[", PreviousSession),
        application_shortcut!("Sessions", "Next session", "]", NextSession),
        application_shortcut!(
            "Sessions",
            "Show archived sessions",
            "shift-a",
            ToggleArchivedSessions
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
        shortcut!(
            "Workspace",
            "Chat and composer",
            "f1",
            FocusComposer,
            None,
            false
        ),
        application_shortcut!("Workspace", "Open Neovim", "e", ShowEditor),
        shortcut!("Workspace", "Open Neovim", "f2", ShowEditor, None, false),
        application_shortcut!("Workspace", "Open terminal", "j", ShowTerminal),
        shortcut!(
            "Workspace",
            "Open terminal",
            "f3",
            ShowTerminal,
            None,
            false
        ),
        Shortcut {
            section: "Transcript",
            label: "Copy visual selection",
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
            "Apply queued steer, double-Esc aborts",
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
            None,
            false
        ),
        application_shortcut!("Application", "Keyboard shortcuts", "/", ShowKeybindings),
        shortcut!(
            "Application",
            "Keyboard shortcuts",
            application_key("?"),
            ShowKeybindings,
            application_context(),
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
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        ApplicationModifier, application_key, bindings, parse_application_modifier, platform_key,
        registry,
    };

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
    fn application_modifier_configuration_accepts_familiar_names() {
        assert_eq!(
            parse_application_modifier(Some("command")),
            ApplicationModifier::Command
        );
        assert_eq!(
            parse_application_modifier(Some("meta")),
            ApplicationModifier::Super
        );
        assert_eq!(
            parse_application_modifier(Some("control")),
            ApplicationModifier::Control
        );
        assert_eq!(
            parse_application_modifier(Some("option")),
            ApplicationModifier::Alt
        );
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
    fn application_shortcuts_use_the_configured_modifier_globally() {
        let shortcuts = registry();
        for (label, suffix) in [
            ("New session", "t"),
            ("Chat and composer", "l"),
            ("Open Neovim", "e"),
            ("Open terminal", "j"),
            ("Open action picker", "k"),
        ] {
            let keystroke = application_key(suffix);
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
            shortcut.label == "Copy visual selection" && shortcut.keystroke == platform!("c")
        }));
        assert!(shortcuts.iter().any(|shortcut| {
            shortcut.label == "Copy selection" && shortcut.keystroke == platform!("c")
        }));
    }
}
