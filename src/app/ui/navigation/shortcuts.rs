//! Shared command definitions for routing, help, and workspace hints.
use crate::app::views::transcript::list::KeyboardCommand;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Composer,
    Editor,
    Terminal,
    RelativeSession(isize),
    Session(usize),
    SearchSessions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Prefix {
    Space,
    G,
}

impl Prefix {
    pub(super) fn from_key(key: &str) -> Option<Self> {
        match key {
            "space" => Some(Self::Space),
            "g" => Some(Self::G),
            _ => None,
        }
    }

    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::Space => leader_hint(),
            Self::G => "g · g transcript top · Esc cancel",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Scroll {
    Start,
    End,
    Lines(f32),
    Pages(f32),
}

impl Scroll {
    pub(super) fn cursor(self) -> KeyboardCommand {
        match self {
            Self::Start => KeyboardCommand::Start,
            Self::End => KeyboardCommand::End,
            Self::Lines(lines) if lines > 0.0 => KeyboardCommand::Down,
            Self::Lines(_) => KeyboardCommand::Up,
            Self::Pages(pages) => KeyboardCommand::Page(pages),
        }
    }
}

const COMMANDS: &[(&str, &str, Command)] = &[
    ("i", "Focus composer", Command::Composer),
    ("a", "Focus composer (alias)", Command::Composer),
    ("/", "Search sessions", Command::SearchSessions),
    ("e", "Open editor", Command::Editor),
    ("t", "Open terminal", Command::Terminal),
    ("space j", "Next session", Command::RelativeSession(1)),
    ("space k", "Previous session", Command::RelativeSession(-1)),
];

const SCROLLS: &[(&str, &str, Scroll)] = &[
    ("g g", "Transcript top", Scroll::Start),
    ("G", "Transcript end (normal: follow latest)", Scroll::End),
    ("j", "Cursor down one rendered line", Scroll::Lines(1.0)),
    ("k", "Cursor up one rendered line", Scroll::Lines(-1.0)),
    ("ctrl-f", "Page down", Scroll::Pages(1.0)),
    ("ctrl-b", "Page up", Scroll::Pages(-1.0)),
    ("ctrl-d", "Half-page down", Scroll::Pages(0.5)),
    ("ctrl-u", "Half-page up", Scroll::Pages(-0.5)),
];

const CURSOR_COMMANDS: &[(&str, &str, KeyboardCommand)] = &[
    ("h", "Previous character", KeyboardCommand::Left),
    ("l", "Next character", KeyboardCommand::Right),
    ("w", "Next word", KeyboardCommand::WordForward),
    ("b", "Previous word", KeyboardCommand::WordBackward),
    (
        "v",
        "Toggle character selection",
        KeyboardCommand::Visual(false),
    ),
    (
        "V",
        "Toggle rendered-line selection",
        KeyboardCommand::Visual(true),
    ),
    (
        "y",
        "Copy selection and return to normal",
        KeyboardCommand::Yank,
    ),
    (
        "escape",
        "Clear selection / cancel pending sequence",
        KeyboardCommand::Cancel,
    ),
];

pub(crate) fn command_key(command: Command) -> &'static str {
    COMMANDS
        .iter()
        .find(|(_, _, candidate)| *candidate == command)
        .expect("command with a workspace hint")
        .0
}

/// Shared with the status strip; adding a leader command updates both surfaces.
pub(crate) fn leader_hint() -> &'static str {
    static HINT: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let commands = COMMANDS
            .iter()
            .filter_map(|(key, label, _)| {
                key.strip_prefix("space ")
                    .map(|key| format!("{key} {}", label.to_lowercase()))
            })
            .collect::<Vec<_>>()
            .join(" · ");
        format!("SPACE · {commands} · Esc cancel")
    });
    &HINT
}

/// Section, key sequence, description. Sequences are individual keycaps in help.
pub(crate) fn help_shortcuts() -> Vec<(&'static str, String, &'static str)> {
    let mut rows = vec![(
        "From anywhere",
        "ctrl-g".into(),
        "Activate app keys for 1 second (no focus change)",
    )];
    if cfg!(target_os = "macos") {
        rows.push((
            "From anywhere",
            "cmd-g".into(),
            "Activate app keys (macOS alias)",
        ));
        rows.push((
            "From anywhere",
            "cmd-g cmd-g".into(),
            "Return to chat normal (macOS alias)",
        ));
    }
    rows.push((
        "From anywhere",
        "ctrl-g ctrl-g".into(),
        "Return to chat normal mode",
    ));
    rows.push((
        "From anywhere",
        "ctrl-g 2".into(),
        "Jump to session 2 (0–9 supported)",
    ));
    rows.extend(
        COMMANDS
            .iter()
            .filter(|(_, _, command)| {
                matches!(
                    command,
                    Command::Editor | Command::Terminal | Command::RelativeSession(_)
                )
            })
            .map(|(key, label, _)| ("From anywhere", format!("ctrl-g {key}"), *label)),
    );
    rows.extend([
        ("Chat", "ctrl-k".into(), "Focus transcript (normal mode)"),
        ("Chat", "ctrl-j".into(), "Focus composer"),
    ]);
    rows.extend(
        COMMANDS
            .iter()
            .map(|(key, label, _)| ("Chat normal", (*key).into(), *label)),
    );
    rows.push(("Chat normal", "0".into(), "First unsubmitted draft"));
    for number in 1..=9 {
        rows.push((
            "Chat normal",
            number.to_string(),
            "Jump to numbered session",
        ));
    }
    rows.extend(
        SCROLLS
            .iter()
            .map(|(key, label, _)| ("Chat normal / visual", (*key).into(), *label)),
    );
    rows.extend(CURSOR_COMMANDS.iter().map(|(key, label, command)| {
        let section = if *command == KeyboardCommand::Yank {
            "Chat visual"
        } else {
            "Chat normal / visual"
        };
        (section, (*key).into(), *label)
    }));
    rows
}

/// True selects transcript normal mode; false selects the composer.
pub(super) fn chat_focus_key(key: &str, modifiers: gpui::Modifiers) -> Option<bool> {
    if modifiers
        != (gpui::Modifiers {
            control: true,
            ..Default::default()
        })
    {
        return None;
    }
    match key {
        "k" => Some(true),
        "j" => Some(false),
        _ => None,
    }
}

pub(super) fn normal_command(key: &str, prefix: Option<Prefix>) -> Option<Command> {
    if prefix.is_none()
        && matches!(
            key,
            "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
        )
    {
        return Some(Command::Session(key.parse().expect("single digit")));
    }
    COMMANDS.iter().find_map(|(sequence, _, command)| {
        let suffix = match prefix {
            Some(Prefix::Space) => sequence.strip_prefix("space "),
            Some(Prefix::G) => None,
            None => Some(*sequence),
        };
        (suffix == Some(key)).then_some(*command)
    })
}

pub(super) fn transcript_scroll(
    key: &str,
    modifiers: gpui::Modifiers,
    prefix: Option<Prefix>,
) -> Option<Scroll> {
    let unmodified = !modifiers.modified();
    if prefix == Some(Prefix::G) && key == "g" && unmodified {
        return Some(Scroll::Start);
    }
    if key == "g"
        && modifiers
            == (gpui::Modifiers {
                shift: true,
                ..Default::default()
            })
    {
        return Some(Scroll::End);
    }
    let plain = unmodified && prefix.is_none();
    let control = modifiers
        == (gpui::Modifiers {
            control: true,
            ..Default::default()
        });
    SCROLLS.iter().find_map(|(sequence, _, scroll)| {
        ((plain && *sequence == key) || (control && sequence.strip_prefix("ctrl-") == Some(key)))
            .then_some(*scroll)
    })
}

/// Cursor commands belong only to the transcript's explicit keyboard owner.
pub(super) fn keyboard_command(
    key: &str,
    modifiers: gpui::Modifiers,
    prefix: Option<Prefix>,
) -> Option<KeyboardCommand> {
    use KeyboardCommand::*;
    if let Some(scroll) = transcript_scroll(key, modifiers, prefix) {
        return Some(scroll.cursor());
    }
    if key == "escape" && !modifiers.modified() {
        return Some(Cancel);
    }
    if prefix.is_some() {
        return None;
    }
    if key == "c"
        && modifiers
            == (gpui::Modifiers {
                platform: cfg!(target_os = "macos"),
                control: !cfg!(target_os = "macos"),
                ..Default::default()
            })
    {
        return Some(Copy);
    }
    let key = if key == "v"
        && modifiers
            == (gpui::Modifiers {
                shift: true,
                ..Default::default()
            }) {
        "V"
    } else if modifiers.modified() {
        return None;
    } else {
        key
    };
    CURSOR_COMMANDS
        .iter()
        .find_map(|(candidate, _, command)| (*candidate == key).then_some(*command))
}
