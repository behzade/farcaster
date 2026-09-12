use crate::app::ui::keyboard::CopySelection;
use crate::app::views::dialogs::send_to_chat::{NextCodeDestination, PreviousCodeDestination};
use crate::app::workspace::{CycleWorkspaceBackward, CycleWorkspaceForward};
use crate::app::{APP_SHORTCUT_CONTEXT, TRANSCRIPT_SELECTION_KEY_CONTEXT};
use crate::app::{
    AbortRun, AddProject, CloseCurrent, ComposerCompletionNext, ComposerCompletionPrevious,
    ComposerEscape, ComposerHistoryNext, ComposerHistoryPrevious, DismissSurface, FocusComposer,
    NewSession, NextSession, OVERLAY_KEY_CONTEXT, PICKER_KEY_CONTEXT, PickerBack, PreviousSession,
    QuitApplication, ShowActionPicker, ShowEditor, ShowKeybindings, ShowTerminal, ShowWorkGraph,
    SubmitFollowUp, SwitchSession0, SwitchSession1, SwitchSession2, SwitchSession3, SwitchSession4,
    SwitchSession5, SwitchSession6, SwitchSession7, SwitchSession8, SwitchSession9, WorkBack,
    WorkCreateIssue, WorkDismiss, WorkFocusSearch, WorkNextIssue, WorkPreviousIssue,
};
use crate::app::{WORKGRAPH_KEY_CONTEXT, WORKGRAPH_NAV_KEY_CONTEXT};
use gpui::{Action as _, KeyBinding, Unbind};
use gpui_base::actions::{SelectDown, SelectUp};

const COMPOSER_COMPLETION_CONTEXT: &str = "(FarcasterComposer && Completions) > Input";
const PICKER_NAVIGATION_CONTEXT: &str =
    "(PiPicker > Input) || (FarcasterSendToChat > List > Input)";

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
    pub show_in_picker: bool,
    pub binding: KeyBinding,
}

impl Shortcut {
    fn in_picker(mut self, show: bool) -> Self {
        self.show_in_picker = show;
        self
    }
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
            show_in_picker: false,
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
    let mut aliases = Vec::new();
    macro_rules! application_shortcut {
        ($section:literal, $label:literal, $key:literal, $action:expr) => {
            application_shortcut!($section, $label, $key, $action, true)
        };
        ($section:literal, $label:literal, $key:literal, $action:expr, $show:expr) => {{
            if cfg!(target_os = "linux") && prefix == "ctrl" {
                aliases.push(shortcut!(
                    $section,
                    $label,
                    concat!("super-", $key),
                    $action,
                    Some(APP_SHORTCUT_CONTEXT),
                    false
                ));
            }
            shortcut!(
                $section,
                $label,
                format!("{prefix}-{}", $key),
                $action,
                Some(APP_SHORTCUT_CONTEXT),
                $show
            )
            .in_picker($show)
        }};
    }
    let session_prefix = if prefix == "cmd" { "cmd" } else { "super" };
    macro_rules! session_shortcut {
        ($label:literal, $key:literal, $action:expr) => {{
            aliases.push(shortcut!(
                "Sessions",
                $label,
                concat!("ctrl-", $key),
                $action,
                Some(APP_SHORTCUT_CONTEXT),
                false
            ));
            shortcut!(
                "Sessions",
                $label,
                format!("{session_prefix}-{}", $key),
                $action,
                Some("FarcasterApp")
            )
        }};
    }
    let mut shortcuts = vec![
        application_shortcut!(
            "Transcript",
            "Increase transcript font size",
            "=",
            crate::app::IncreaseTranscriptFontSize
        ),
        application_shortcut!(
            "Transcript",
            "Increase transcript font size",
            "+",
            crate::app::IncreaseTranscriptFontSize,
            false
        ),
        application_shortcut!(
            "Transcript",
            "Decrease transcript font size",
            "-",
            crate::app::DecreaseTranscriptFontSize
        ),
        application_shortcut!("Sessions", "New session", "n", NewSession),
        session_shortcut!("Open first unsubmitted draft", "0", SwitchSession0),
        session_shortcut!("Open session 1", "1", SwitchSession1),
        session_shortcut!("Open session 2", "2", SwitchSession2),
        session_shortcut!("Open session 3", "3", SwitchSession3),
        session_shortcut!("Open session 4", "4", SwitchSession4),
        session_shortcut!("Open session 5", "5", SwitchSession5),
        session_shortcut!("Open session 6", "6", SwitchSession6),
        session_shortcut!("Open session 7", "7", SwitchSession7),
        session_shortcut!("Open session 8", "8", SwitchSession8),
        session_shortcut!("Open session 9", "9", SwitchSession9),
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
        application_shortcut!(
            "Configuration",
            "Set harness",
            "shift-h",
            crate::app::SetHarness
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
            "Dismiss dialog; close surface or draft; archive session",
            "w",
            CloseCurrent
        ),
        Shortcut {
            section: "Composer",
            label: "Previous prompt from first line (no suggestions)",
            keystroke: "up".into(),
            show_in_help: true,
            show_in_picker: false,
            binding: KeyBinding::new(
                "up",
                ComposerHistoryPrevious,
                Some("FarcasterComposer > Input"),
            ),
        },
        Shortcut {
            section: "Composer",
            label: "Next prompt from last line while browsing history",
            keystroke: "down".into(),
            show_in_help: true,
            show_in_picker: false,
            binding: KeyBinding::new(
                "down",
                ComposerHistoryNext,
                Some("FarcasterComposer > Input"),
            ),
        },
        shortcut!(
            "Composer",
            "Previous completion",
            "ctrl-p",
            ComposerCompletionPrevious,
            Some(COMPOSER_COMPLETION_CONTEXT)
        ),
        shortcut!(
            "Composer",
            "Next completion",
            "ctrl-n",
            ComposerCompletionNext,
            Some(COMPOSER_COMPLETION_CONTEXT)
        ),
        shortcut!(
            "Workspace",
            "Chat and composer",
            "f1",
            FocusComposer,
            Some(APP_SHORTCUT_CONTEXT),
            false
        )
        .in_picker(true),
        shortcut!(
            "Workspace",
            "Open transcript in Neovim",
            "ctrl-g v",
            crate::app::OpenTranscriptScratch,
            Some(APP_SHORTCUT_CONTEXT)
        )
        .in_picker(true),
        shortcut!(
            "Workspace",
            "Open Neovim",
            "f2",
            ShowEditor,
            Some(APP_SHORTCUT_CONTEXT)
        )
        .in_picker(true),
        shortcut!(
            "Workspace",
            "Open terminal",
            "f3",
            ShowTerminal,
            Some(APP_SHORTCUT_CONTEXT)
        )
        .in_picker(true),
        shortcut!(
            "Workspace",
            "Next workspace surface",
            "ctrl-tab",
            CycleWorkspaceForward,
            Some("FarcasterApp")
        )
        .in_picker(true),
        shortcut!(
            "Workspace",
            "Previous workspace surface",
            "ctrl-shift-tab",
            CycleWorkspaceBackward,
            Some("FarcasterApp")
        )
        .in_picker(true),
        Shortcut {
            section: "Transcript",
            label: "Copy transcript selection",
            keystroke: platform!("c").into(),
            show_in_help: false,
            show_in_picker: false,
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
            show_in_picker: false,
            binding: KeyBinding::new(
                platform!("c"),
                CopySelection,
                Some("FarcasterComposer > Input"),
            ),
        },
        shortcut!(
            "Composer",
            "Send prompt; queue follow-up during a run (no suggestions)",
            "tab",
            SubmitFollowUp,
            Some("(FarcasterComposer && !Completions) > Input")
        ),
        shortcut!(
            "Composer",
            "Next completion",
            "tab",
            ComposerCompletionNext,
            Some(COMPOSER_COMPLETION_CONTEXT)
        ),
        shortcut!(
            "Composer",
            "Previous completion",
            "shift-tab",
            ComposerCompletionPrevious,
            Some(COMPOSER_COMPLETION_CONTEXT)
        ),
        application_shortcut!("Run", "Abort current run", ".", AbortRun),
        Shortcut {
            section: "Composer",
            label: "Send pending input; double-Esc aborts",
            keystroke: "escape".into(),
            show_in_help: true,
            show_in_picker: false,
            // Raw key handling needs `KeyDownEvent::is_held`, which action dispatch omits.
            binding: KeyBinding::new(
                "escape",
                Unbind(ComposerEscape.name().into()),
                Some("FarcasterComposer > Input"),
            ),
        },
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
            "Back to all plans",
            "backspace",
            WorkBack,
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
        application_shortcut!(
            "Application",
            "Open action picker",
            "shift-p",
            ShowActionPicker
        )
        .in_picker(false),
        shortcut!(
            "Application",
            "Open action picker",
            "f4",
            ShowActionPicker,
            Some(APP_SHORTCUT_CONTEXT),
            false
        ),
        application_shortcut!(
            "Sessions",
            "Focus session search",
            "/",
            crate::app::FocusSessionSearch
        ),
        application_shortcut!(
            "Application",
            "Keyboard shortcuts",
            "shift-/",
            ShowKeybindings
        ),
        application_shortcut!(
            "Application",
            "Keyboard shortcuts",
            "?",
            ShowKeybindings,
            false
        ),
        shortcut!(
            "Send to chat",
            "Previous destination",
            "ctrl-p",
            PreviousCodeDestination,
            Some("FarcasterSendToChat")
        ),
        shortcut!(
            "Send to chat",
            "Next destination",
            "ctrl-n",
            NextCodeDestination,
            Some("FarcasterSendToChat")
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
            show_in_picker: false,
            binding: KeyBinding::new("ctrl-p", SelectUp, Some(PICKER_NAVIGATION_CONTEXT)),
        },
        Shortcut {
            section: "Application",
            label: "Next picker item",
            keystroke: "ctrl-n".into(),
            show_in_help: false,
            show_in_picker: false,
            binding: KeyBinding::new("ctrl-n", SelectDown, Some(PICKER_NAVIGATION_CONTEXT)),
        },
        shortcut!(
            "Application",
            "Previous picker item",
            "shift-tab",
            SelectUp,
            Some("PiPicker > Input"),
            false
        ),
        shortcut!(
            "Application",
            "Next picker item",
            "tab",
            SelectDown,
            Some("PiPicker > Input"),
            false
        ),
        shortcut!(
            "Application",
            "Back in action picker",
            "alt-left",
            crate::app::PickerNavigateBack,
            Some(PICKER_KEY_CONTEXT)
        ),
        Shortcut {
            section: "Application",
            label: "Back in action picker when search is empty",
            keystroke: "backspace".into(),
            show_in_help: false,
            show_in_picker: false,
            binding: KeyBinding::new("backspace", PickerBack, Some("PiPicker > Input")),
        },
        Shortcut {
            section: "Application",
            label: "Close action picker",
            keystroke: "escape".into(),
            show_in_help: false,
            show_in_picker: false,
            binding: KeyBinding::new("escape", DismissSurface, Some(PICKER_KEY_CONTEXT)),
        },
        application_shortcut!("Application", "Quit", "q", QuitApplication),
    ];
    shortcuts.extend(aliases);
    for modifier in ["ctrl", "cmd", "super"] {
        shortcuts.extend([
            shortcut!(
                "Transcript",
                "Next session (including archived)",
                format!("{modifier}-j"),
                crate::app::NextTranscriptSession,
                Some(crate::app::CHAT_SHORTCUT_CONTEXT)
            ),
            shortcut!(
                "Transcript",
                "Previous session (including archived)",
                format!("{modifier}-k"),
                crate::app::PreviousTranscriptSession,
                Some(crate::app::CHAT_SHORTCUT_CONTEXT)
            ),
        ]);
    }
    shortcuts
}

#[cfg(test)]
#[path = "keybindings_tests.rs"]
mod tests;
