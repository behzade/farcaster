//! Shared command definitions for routing, help, and workspace hints.
use crate::app::views::transcript::list::KeyboardCommand;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Editor,
    Terminal,
    RelativeSession(isize),
    Session(usize),
    SearchSessions,
    NewSession,
    Close,
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
    ("/", "Search sessions", Command::SearchSessions),
    ("e", "Open editor", Command::Editor),
    ("t", "Open terminal", Command::Terminal),
    ("n", "New session", Command::NewSession),
    ("w", "Close surface or session", Command::Close),
    ("space j", "Next session", Command::RelativeSession(1)),
    ("space k", "Previous session", Command::RelativeSession(-1)),
];

const SCROLLS: &[(&str, &str, Scroll)] = &[
    ("g g", "Transcript top", Scroll::Start),
    ("G", "Transcript end (normal: follow latest)", Scroll::End),
    ("j", "Cursor down one logical line", Scroll::Lines(1.0)),
    ("k", "Cursor up one logical line", Scroll::Lines(-1.0)),
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
    ("e", "End of word", KeyboardCommand::WordEnd),
    ("W", "Next WORD", KeyboardCommand::BigWordForward),
    ("B", "Previous WORD", KeyboardCommand::BigWordBackward),
    ("E", "End of WORD", KeyboardCommand::BigWordEnd),
    ("0", "Line start", KeyboardCommand::LineStart),
    ("^", "First nonblank", KeyboardCommand::FirstNonblank),
    ("$", "Line end", KeyboardCommand::LineEnd),
    ("|", "Line start", KeyboardCommand::LineStart),
    (
        "+",
        "Next line first nonblank",
        KeyboardCommand::FirstLine(1),
    ),
    (
        "-",
        "Previous line first nonblank",
        KeyboardCommand::FirstLine(-1),
    ),
    (
        "enter",
        "Next line first nonblank",
        KeyboardCommand::FirstLine(1),
    ),
    ("_", "First nonblank", KeyboardCommand::FirstNonblank),
    ("{", "Previous paragraph", KeyboardCommand::Paragraph(false)),
    ("}", "Next paragraph", KeyboardCommand::Paragraph(true)),
    ("(", "Previous sentence", KeyboardCommand::Sentence(false)),
    (")", "Next sentence", KeyboardCommand::Sentence(true)),
    ("%", "Matching bracket", KeyboardCommand::MatchBracket),
    ("H", "Viewport top", KeyboardCommand::Viewport(0)),
    ("M", "Viewport middle", KeyboardCommand::Viewport(1)),
    ("L", "Viewport bottom", KeyboardCommand::Viewport(2)),
    (
        ";",
        "Repeat character find",
        KeyboardCommand::RepeatFind(false),
    ),
    (
        ",",
        "Reverse character find",
        KeyboardCommand::RepeatFind(true),
    ),
    ("n", "Next search match", KeyboardCommand::SearchNext(false)),
    (
        "N",
        "Previous search match",
        KeyboardCommand::SearchNext(true),
    ),
    (
        "*",
        "Search word forward",
        KeyboardCommand::SearchWord(true),
    ),
    (
        "#",
        "Search word backward",
        KeyboardCommand::SearchWord(false),
    ),
    (
        "o",
        "Swap visual selection ends",
        KeyboardCommand::SwapAnchor,
    ),
    ("left", "Previous character", KeyboardCommand::Left),
    ("right", "Next character", KeyboardCommand::Right),
    ("up", "Previous line", KeyboardCommand::Up),
    ("down", "Next line", KeyboardCommand::Down),
    ("home", "Line start", KeyboardCommand::LineStart),
    ("end", "Line end", KeyboardCommand::LineEnd),
    ("pageup", "Page up", KeyboardCommand::Page(-1.0)),
    ("pagedown", "Page down", KeyboardCommand::Page(1.0)),
    (
        "v",
        "Toggle character selection",
        KeyboardCommand::Visual(false),
    ),
    (
        "V",
        "Toggle logical-line selection",
        KeyboardCommand::Visual(true),
    ),
    (
        "y",
        "Copy selection and leave visual selection",
        KeyboardCommand::Yank,
    ),
    (
        "escape",
        "Clear selection / cancel pending sequence",
        KeyboardCommand::Cancel,
    ),
];

pub(crate) fn command_key(command: Command) -> &'static str {
    match command {
        Command::Editor => return "Ctrl-G e",
        Command::Terminal => return "Ctrl-G t",
        Command::NewSession => return "Ctrl-G n",
        Command::Close => return "Ctrl-G w",
        _ => {}
    }
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
            "App-owned contexts",
            "cmd-g".into(),
            "Focus chat composer",
        ));
    }
    rows.push((
        "From anywhere",
        "ctrl-g ctrl-g".into(),
        "Return to chat composer",
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
                    Command::Editor
                        | Command::Terminal
                        | Command::NewSession
                        | Command::Close
                        | Command::RelativeSession(_)
                        | Command::SearchSessions
                )
            })
            .map(|(key, label, _)| ("From anywhere", format!("ctrl-g {key}"), *label)),
    );
    rows.extend([
        ("Chat", "ctrl-k".into(), "Focus transcript"),
        ("Chat", "ctrl-j".into(), "Focus composer"),
        ("Agent confirmation", "n".into(), "No / deny"),
        ("Agent confirmation", "y".into(), "Yes / allow"),
    ]);
    rows.extend(
        COMMANDS
            .iter()
            .filter(|(_, _, command)| matches!(command, Command::RelativeSession(_)))
            .map(|(key, label, _)| ("Transcript", (*key).into(), *label)),
    );
    rows.extend([
        ("Transcript", "1–9".into(), "Switch session"),
        (
            "Transcript",
            "z t / z z / z b".into(),
            "Align cursor line at top / center / bottom",
        ),
        (
            "Transcript",
            "g e".into(),
            "Previous word end (g E for WORD)",
        ),
        (
            "Transcript",
            "g j".into(),
            "Next wrapped line (g k for previous)",
        ),
        (
            "Transcript",
            "g 0".into(),
            "Wrapped-line start (g ^ / g $ for nonblank / end)",
        ),
        ("Transcript", "g _".into(), "Last nonblank"),
        (
            "Transcript",
            "f / F / t / T".into(),
            "Find / till next character, forward / backward",
        ),
        (
            "Transcript",
            "/ / ?".into(),
            "Search rendered text forward / backward; Enter confirms",
        ),
    ]);
    rows.extend(
        SCROLLS
            .iter()
            .map(|(key, label, _)| ("Transcript", (*key).into(), *label)),
    );
    rows.extend(CURSOR_COMMANDS.iter().map(|(key, label, command)| {
        let section = if *command == KeyboardCommand::Yank {
            "Transcript visual"
        } else {
            "Transcript"
        };
        (section, (*key).into(), *label)
    }));
    rows
}

/// True selects the transcript; false selects the composer.
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
    activated_command(key, prefix).filter(|command| {
        matches!(
            command,
            Command::RelativeSession(_) | Command::Session(1..=9)
        )
    })
}

pub(super) fn activated_command(key: &str, prefix: Option<Prefix>) -> Option<Command> {
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
    let key = plain_key(key, modifiers)?;

    CURSOR_COMMANDS
        .iter()
        .find_map(|(candidate, _, command)| (*candidate == key.as_str()).then_some(*command))
}

/// GPUI normalizes uppercase letters to a lowercase key plus Shift. Punctuation
/// can arrive either already shifted or as the physical key plus Shift.
pub(super) fn plain_key(key: &str, modifiers: gpui::Modifiers) -> Option<String> {
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return None;
    }
    if !modifiers.shift {
        return Some(key.to_owned());
    }
    Some(match key {
        "4" => "$".into(),
        "6" => "^".into(),
        "5" => "%".into(),
        "8" => "*".into(),
        "3" => "#".into(),
        "9" => "(".into(),
        "0" => ")".into(),
        "[" => "{".into(),
        "]" => "}".into(),
        "-" => "_".into(),
        "=" => "+".into(),
        "\\" => "|".into(),
        "/" => "?".into(),
        _ => key.to_uppercase(),
    })
}
