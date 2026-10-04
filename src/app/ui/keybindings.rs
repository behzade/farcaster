use crate::app::WORKGRAPH_KEY_CONTEXT;
use crate::app::ui::keyboard::CopySelection;
use crate::app::views::dialogs::send_to_chat::{NextCodeDestination, PreviousCodeDestination};
use crate::app::workspace::{CycleWorkspaceBackward, CycleWorkspaceForward};
use crate::app::{
    AbortRun, AddProject, CloseCurrent, ComposerCompletionNext, ComposerCompletionPrevious,
    ComposerEscape, ComposerHistoryNext, ComposerHistoryPrevious, DismissSurface, FocusComposer,
    NewSession, NextSession, OVERLAY_KEY_CONTEXT, PICKER_KEY_CONTEXT, PickerBack, PreviousSession,
    QuitApplication, ShowActionPicker, ShowEditor, ShowKeybindings, ShowTerminal, ShowWorkGraph,
    SubmitFollowUp, SwitchSession0, SwitchSession1, SwitchSession2, SwitchSession3, SwitchSession4,
    SwitchSession5, SwitchSession6, SwitchSession7, SwitchSession8, SwitchSession9, WorkBack,
    WorkCreateIssue, WorkDismiss, WorkFocusSearch, WorkNextIssue, WorkPreviousIssue,
};
use crate::app::{COMPOSER_KEY_CONTEXT, TRANSCRIPT_SELECTION_KEY_CONTEXT};
use gpui::{Action as _, KeyBinding, KeyBindingContextPredicate as Predicate, Unbind};
use gpui_base::actions::{SelectDown, SelectUp};

fn binding(key: &str, action: Box<dyn gpui::Action>, condition: Predicate) -> KeyBinding {
    KeyBinding::load(
        key,
        action,
        Some(std::rc::Rc::new(condition)),
        false,
        None,
        &gpui::DummyKeyboardMapper,
    )
    .expect("registered shortcut must have a valid keystroke")
}

fn named(name: &'static str) -> Predicate {
    Predicate::Identifier(name.into())
}

fn input_in(parent: Predicate) -> Predicate {
    Predicate::Descendant(Box::new(parent), Box::new(named("Input")))
}

pub(crate) fn app_condition() -> Predicate {
    Predicate::And(
        Box::new(named("FarcasterApp")),
        Box::new(Predicate::Equal("input".into(), "app".into())),
    )
}

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
    pub keystroke: String,
    pub condition: Predicate,
    pub show_in_help: bool,
}

impl Shortcut {
    fn new(key: impl Into<String>, condition: Predicate) -> Self {
        Self {
            keystroke: key.into(),
            condition,
            show_in_help: true,
        }
    }

    fn hidden(mut self) -> Self {
        self.show_in_help = false;
        self
    }
}

pub(crate) struct Command {
    pub action: Box<dyn gpui::Action>,
    pub section: &'static str,
    pub label: &'static str,
    pub show_in_help: bool,
    pub show_in_picker: bool,
    // The first binding is the shortcut shown in the action picker.
    pub bindings: Vec<Shortcut>,
}

impl Command {
    fn new(
        action: impl gpui::Action,
        section: &'static str,
        label: &'static str,
        bindings: impl IntoIterator<Item = Shortcut>,
    ) -> Self {
        Self {
            action: Box::new(action),
            section,
            label,
            show_in_help: true,
            show_in_picker: false,
            bindings: bindings.into_iter().collect(),
        }
    }

    fn with_bindings(mut self, bindings: impl IntoIterator<Item = Shortcut>) -> Self {
        self.bindings.extend(bindings);
        self
    }

    fn in_picker(mut self) -> Self {
        self.show_in_picker = true;
        self
    }

    fn hide_from_help(mut self) -> Self {
        self.show_in_help = false;
        self
    }

    pub(crate) fn into_bindings(self) -> impl Iterator<Item = KeyBinding> {
        self.bindings.into_iter().map(move |shortcut| {
            binding(
                &shortcut.keystroke,
                self.action.boxed_clone(),
                shortcut.condition,
            )
        })
    }
}

fn app_keys(prefix: &str, suffix: &str) -> Vec<Shortcut> {
    let mut keys = vec![Shortcut::new(format!("{prefix}-{suffix}"), app_condition())];
    if cfg!(target_os = "linux") && prefix == "ctrl" {
        keys.push(Shortcut::new(format!("super-{suffix}"), app_condition()).hidden());
    }
    keys
}

fn session_keys(prefix: &str, number: u8) -> [Shortcut; 2] {
    let (alias, primary_condition, alias_condition) = if prefix == "cmd" {
        ("ctrl", named("FarcasterApp"), app_condition())
    } else {
        ("super", app_condition(), named("FarcasterApp"))
    };
    [
        Shortcut::new(format!("{prefix}-{number}"), primary_condition),
        Shortcut::new(format!("{alias}-{number}"), alias_condition).hidden(),
    ]
}

fn navigation_keys(prefix: &str, key: &str, chat: &Predicate) -> Vec<Shortcut> {
    let mut modifiers = ["ctrl", "cmd", "super"];
    modifiers.sort_by_key(|modifier| *modifier != prefix);
    modifiers
        .into_iter()
        .map(|modifier| {
            let condition = chat.clone();
            Shortcut::new(format!("{modifier}-{key}"), condition)
        })
        .collect()
}

pub(crate) fn bindings() -> Vec<KeyBinding> {
    let mut bindings = registry()
        .into_iter()
        .flat_map(Command::into_bindings)
        .collect::<Vec<_>>();
    bindings.extend([
        binding("tab", Box::new(Unbind("root::Tab".into())), named("Root")),
        binding(
            "shift-tab",
            Box::new(Unbind("root::TabPrev".into())),
            named("Root"),
        ),
    ]);
    bindings
}

pub(crate) fn registry() -> Vec<Command> {
    registry_for_platform(platform_key("cmd", "ctrl"))
}

fn registry_for_platform(prefix: &str) -> Vec<Command> {
    let chat = Predicate::And(
        Box::new(app_condition()),
        Box::new(Predicate::Equal("surface".into(), "chat".into())),
    );
    let composer_input = input_in(named(COMPOSER_KEY_CONTEXT));
    let composer_completions = input_in(Predicate::And(
        Box::new(named(COMPOSER_KEY_CONTEXT)),
        Box::new(named("Completions")),
    ));
    let composer_without_completions = input_in(Predicate::And(
        Box::new(named(COMPOSER_KEY_CONTEXT)),
        Box::new(Predicate::Not(Box::new(named("Completions")))),
    ));
    let picker_input = input_in(named(PICKER_KEY_CONTEXT));
    let picker_navigation = Predicate::Or(
        Box::new(picker_input.clone()),
        Box::new(input_in(Predicate::Descendant(
            Box::new(named("FarcasterSendToChat")),
            Box::new(named("List")),
        ))),
    );
    let workgraph_navigation = Predicate::And(
        Box::new(named(WORKGRAPH_KEY_CONTEXT)),
        Box::new(Predicate::Not(Box::new(named("Input")))),
    );

    vec![
        Command::new(
            crate::app::IncreaseTranscriptFontSize,
            "Transcript",
            "Increase transcript font size",
            app_keys(prefix, "="),
        )
        .with_bindings(app_keys(prefix, "+").into_iter().map(Shortcut::hidden))
        .in_picker(),
        Command::new(
            crate::app::DecreaseTranscriptFontSize,
            "Transcript",
            "Decrease transcript font size",
            app_keys(prefix, "-"),
        )
        .in_picker(),
        Command::new(NewSession, "Sessions", "New session", app_keys(prefix, "n")).in_picker(),
        Command::new(
            SwitchSession0,
            "Sessions",
            "Open first draft",
            session_keys(prefix, 0),
        ),
        Command::new(
            SwitchSession1,
            "Sessions",
            "Open session 1",
            session_keys(prefix, 1),
        ),
        Command::new(
            SwitchSession2,
            "Sessions",
            "Open session 2",
            session_keys(prefix, 2),
        ),
        Command::new(
            SwitchSession3,
            "Sessions",
            "Open session 3",
            session_keys(prefix, 3),
        ),
        Command::new(
            SwitchSession4,
            "Sessions",
            "Open session 4",
            session_keys(prefix, 4),
        ),
        Command::new(
            SwitchSession5,
            "Sessions",
            "Open session 5",
            session_keys(prefix, 5),
        ),
        Command::new(
            SwitchSession6,
            "Sessions",
            "Open session 6",
            session_keys(prefix, 6),
        ),
        Command::new(
            SwitchSession7,
            "Sessions",
            "Open session 7",
            session_keys(prefix, 7),
        ),
        Command::new(
            SwitchSession8,
            "Sessions",
            "Open session 8",
            session_keys(prefix, 8),
        ),
        Command::new(
            SwitchSession9,
            "Sessions",
            "Open session 9",
            session_keys(prefix, 9),
        ),
        Command::new(
            AddProject,
            "Sessions",
            "Add project",
            app_keys(prefix, "shift-n"),
        )
        .in_picker(),
        Command::new(
            crate::app::SetSandbox,
            "Configuration",
            "Set sandbox",
            app_keys(prefix, "shift-s"),
        )
        .in_picker(),
        Command::new(
            crate::app::SetRuntime,
            "Configuration",
            "Set provider/model/effort",
            app_keys(prefix, "shift-m"),
        )
        .in_picker(),
        Command::new(
            crate::app::SetHarness,
            "Configuration",
            "Set harness",
            app_keys(prefix, "shift-h"),
        )
        .in_picker(),
        Command::new(
            PreviousSession,
            "Sessions",
            "Previous session",
            app_keys(prefix, "["),
        )
        .in_picker(),
        Command::new(
            NextSession,
            "Sessions",
            "Next session",
            app_keys(prefix, "]"),
        )
        .in_picker(),
        Command::new(
            crate::app::RestoreSession,
            "Sessions",
            "Restore session",
            app_keys(prefix, "shift-a"),
        )
        .in_picker(),
        Command::new(
            CloseCurrent,
            "Sessions",
            "Dismiss dialog; close surface or draft; archive session",
            app_keys(prefix, "w"),
        )
        .in_picker(),
        Command::new(
            ComposerHistoryPrevious,
            "Composer",
            "Previous prompt from first line (no suggestions)",
            [Shortcut::new("up", composer_input.clone())],
        ),
        Command::new(
            ComposerHistoryNext,
            "Composer",
            "Next prompt from last line while browsing history",
            [Shortcut::new("down", composer_input.clone())],
        ),
        Command::new(
            ComposerCompletionPrevious,
            "Composer",
            "Previous completion",
            [
                Shortcut::new("ctrl-p", composer_completions.clone()),
                Shortcut::new("shift-tab", composer_completions.clone()),
            ],
        ),
        Command::new(
            ComposerCompletionNext,
            "Composer",
            "Next completion",
            [
                Shortcut::new("ctrl-n", composer_completions.clone()),
                Shortcut::new("tab", composer_completions),
            ],
        ),
        Command::new(
            FocusComposer,
            "Workspace",
            "Chat and composer",
            [Shortcut::new("f1", app_condition())],
        )
        .hide_from_help()
        .in_picker(),
        Command::new(
            crate::app::OpenTranscriptScratch,
            "Workspace",
            "Open transcript in Neovim",
            [Shortcut::new("ctrl-g v", app_condition())],
        )
        .in_picker(),
        Command::new(
            ShowEditor,
            "Workspace",
            "Open Neovim",
            [Shortcut::new("f2", app_condition())],
        )
        .in_picker(),
        Command::new(
            ShowTerminal,
            "Workspace",
            "Open terminal",
            [Shortcut::new("f3", app_condition())],
        )
        .in_picker(),
        Command::new(
            CycleWorkspaceForward,
            "Workspace",
            "Next workspace surface",
            [Shortcut::new("ctrl-tab", named("FarcasterApp"))],
        )
        .in_picker(),
        Command::new(
            CycleWorkspaceBackward,
            "Workspace",
            "Previous workspace surface",
            [Shortcut::new("ctrl-shift-tab", named("FarcasterApp"))],
        )
        .in_picker(),
        Command::new(
            CopySelection,
            "Selection",
            "Copy selection",
            [
                Shortcut::new(
                    format!("{prefix}-c"),
                    named(TRANSCRIPT_SELECTION_KEY_CONTEXT),
                ),
                Shortcut::new(format!("{prefix}-c"), composer_input.clone()),
            ],
        )
        .hide_from_help(),
        Command::new(
            SubmitFollowUp,
            "Composer",
            "Send prompt; queue follow-up during a run (no suggestions)",
            [Shortcut::new("tab", composer_without_completions)],
        ),
        Command::new(AbortRun, "Run", "Abort current run", app_keys(prefix, ".")).in_picker(),
        Command::new(
            Unbind(ComposerEscape.name().into()),
            "Composer",
            "Send pending input; double-Esc aborts",
            [Shortcut::new("escape", composer_input)],
        ),
        Command::new(
            WorkPreviousIssue,
            "Work",
            "Previous node",
            [Shortcut::new("k", workgraph_navigation.clone())],
        ),
        Command::new(
            WorkNextIssue,
            "Work",
            "Next node",
            [Shortcut::new("j", workgraph_navigation.clone())],
        ),
        Command::new(
            WorkFocusSearch,
            "Work",
            "Search plan",
            [Shortcut::new("/", workgraph_navigation.clone())],
        ),
        Command::new(
            WorkCreateIssue,
            "Work",
            "Add plan node",
            [Shortcut::new("c", workgraph_navigation.clone())],
        ),
        Command::new(
            WorkBack,
            "Work",
            "Back to all plans",
            [Shortcut::new("backspace", workgraph_navigation)],
        ),
        Command::new(
            WorkDismiss,
            "Work",
            "Back or clear",
            [Shortcut::new("escape", named(WORKGRAPH_KEY_CONTEXT))],
        ),
        Command::new(
            ShowWorkGraph,
            "Application",
            "Open / close project work",
            app_keys(prefix, "shift-i"),
        )
        .in_picker(),
        Command::new(
            ShowActionPicker,
            "Application",
            "Open action picker",
            app_keys(prefix, "shift-p"),
        )
        .with_bindings([Shortcut::new("f4", app_condition()).hidden()]),
        Command::new(
            crate::app::FocusSessionSearch,
            "Sessions",
            "Focus session search",
            app_keys(prefix, "/"),
        )
        .in_picker(),
        Command::new(
            ShowKeybindings,
            "Application",
            "Keyboard shortcuts",
            app_keys(prefix, "shift-/"),
        )
        .with_bindings(app_keys(prefix, "?").into_iter().map(Shortcut::hidden))
        .in_picker(),
        Command::new(
            PreviousCodeDestination,
            "Send to chat",
            "Previous destination",
            [Shortcut::new("ctrl-p", named("FarcasterSendToChat"))],
        ),
        Command::new(
            NextCodeDestination,
            "Send to chat",
            "Next destination",
            [Shortcut::new("ctrl-n", named("FarcasterSendToChat"))],
        ),
        Command::new(
            DismissSurface,
            "Application",
            "Close dialog",
            [
                Shortcut::new("escape", named(OVERLAY_KEY_CONTEXT)),
                Shortcut::new("escape", named(PICKER_KEY_CONTEXT)).hidden(),
            ],
        ),
        Command::new(
            SelectUp,
            "Application",
            "Previous picker item",
            [
                Shortcut::new("ctrl-p", picker_navigation.clone()),
                Shortcut::new("shift-tab", picker_input.clone()),
            ],
        )
        .hide_from_help(),
        Command::new(
            SelectDown,
            "Application",
            "Next picker item",
            [
                Shortcut::new("ctrl-n", picker_navigation),
                Shortcut::new("tab", picker_input.clone()),
            ],
        )
        .hide_from_help(),
        Command::new(
            crate::app::PickerNavigateBack,
            "Application",
            "Back in action picker",
            [Shortcut::new("alt-left", named(PICKER_KEY_CONTEXT))],
        ),
        Command::new(
            PickerBack,
            "Application",
            "Back in action picker when search is empty",
            [Shortcut::new("backspace", picker_input)],
        )
        .hide_from_help(),
        Command::new(
            QuitApplication,
            "Application",
            "Quit",
            app_keys(prefix, "q"),
        )
        .in_picker(),
        Command::new(
            crate::app::NextTranscriptSession,
            "Transcript",
            "Next session (including archived)",
            navigation_keys(prefix, "j", &chat),
        ),
        Command::new(
            crate::app::PreviousTranscriptSession,
            "Transcript",
            "Previous session (including archived)",
            navigation_keys(prefix, "k", &chat),
        ),
        Command::new(
            crate::app::NextWorker,
            "Workers",
            "Next worker or parent chat",
            navigation_keys(prefix, "shift-j", &chat),
        )
        .in_picker(),
        Command::new(
            crate::app::PreviousWorker,
            "Workers",
            "Previous worker or parent chat",
            navigation_keys(prefix, "shift-k", &chat),
        )
        .in_picker(),
    ]
}

#[cfg(test)]
#[path = "keybindings_tests.rs"]
mod tests;
