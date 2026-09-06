//! Shared command definitions for routing, help, and workspace hints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Composer,
    Editor,
    Terminal,
    RelativeSession(isize),
    Session(usize),
    SearchSessions,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Scroll {
    Lines(f32),
    Pages(f32),
}

const COMMANDS: &[(&str, &str, Command)] = &[
    ("i", "Focus composer", Command::Composer),
    ("a", "Focus composer (alias)", Command::Composer),
    ("/", "Search sessions", Command::SearchSessions),
    ("space e", "Open editor", Command::Editor),
    ("space t", "Open terminal", Command::Terminal),
    ("space j", "Next session", Command::RelativeSession(1)),
    ("space k", "Previous session", Command::RelativeSession(-1)),
];

const SCROLLS: &[(&str, &str, Scroll)] = &[
    ("j", "Scroll transcript down", Scroll::Lines(1.0)),
    ("k", "Scroll transcript up", Scroll::Lines(-1.0)),
    ("ctrl-f", "Page down", Scroll::Pages(1.0)),
    ("ctrl-b", "Page up", Scroll::Pages(-1.0)),
    ("ctrl-d", "Half-page down", Scroll::Pages(0.5)),
    ("ctrl-u", "Half-page up", Scroll::Pages(-0.5)),
];

pub(crate) fn command_key(command: Command) -> &'static str {
    COMMANDS
        .iter()
        .find(|(_, _, candidate)| *candidate == command)
        .expect("command with a workspace hint")
        .0
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
    rows.push((
        "From anywhere",
        "ctrl-g space e".into(),
        "Run Space command (e/t/j/k)",
    ));
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
            .map(|(key, label, _)| ("Chat normal", (*key).into(), *label)),
    );
    rows.push(("Chat normal", "escape".into(), "Cancel pending leader"));
    rows
}

pub(super) fn normal_command(key: &str, leader: bool) -> Option<Command> {
    if !leader
        && matches!(
            key,
            "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
        )
    {
        return Some(Command::Session(key.parse().expect("single digit")));
    }
    COMMANDS.iter().find_map(|(sequence, _, command)| {
        let suffix = if leader {
            sequence.strip_prefix("space ")
        } else {
            Some(*sequence)
        };
        (suffix == Some(key)).then_some(*command)
    })
}

pub(super) fn transcript_scroll(
    key: &str,
    modifiers: gpui::Modifiers,
    leader: bool,
) -> Option<Scroll> {
    let plain = !modifiers.modified() && !leader;
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
