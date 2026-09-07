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
    ("G", "Transcript end (follow latest)", Scroll::End),
    ("j", "Scroll down one line", Scroll::Lines(1.0)),
    ("k", "Scroll up one line", Scroll::Lines(-1.0)),
    ("ctrl-f", "Page down", Scroll::Pages(1.0)),
    ("ctrl-b", "Page up", Scroll::Pages(-1.0)),
    ("ctrl-d", "Half-page down", Scroll::Pages(0.5)),
    ("ctrl-u", "Half-page up", Scroll::Pages(-0.5)),
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

pub(crate) fn help_shortcuts() -> Vec<(&'static str, String, &'static str)> {
    let mut rows = vec![(
        "From anywhere",
        "ctrl-g".into(),
        "Activate app keys for 1 second (no focus change)",
    )];
    if cfg!(target_os = "macos") {
        rows.push(("App-owned contexts", "cmd-g".into(), "Focus chat composer"));
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
            .map(|(key, label, _)| ("From anywhere", format!("ctrl-g {key}"), *label)),
    );
    rows.extend([
        ("Agent confirmation", "n".into(), "No / deny"),
        ("Agent confirmation", "y".into(), "Yes / allow"),
    ]);
    rows.extend(SCROLLS.iter().map(|(key, label, _)| {
        if key.starts_with("ctrl-") {
            ("Chat", (*key).into(), *label)
        } else {
            ("From anywhere", format!("ctrl-g {key}"), *label)
        }
    }));
    rows
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
    chat_scroll(key, modifiers).or_else(|| {
        SCROLLS
            .iter()
            .find_map(|(sequence, _, scroll)| (plain && *sequence == key).then_some(*scroll))
    })
}

/// Direct scrolling is limited to exact Ctrl chords, so typing stays in the composer.
pub(super) fn chat_scroll(key: &str, modifiers: gpui::Modifiers) -> Option<Scroll> {
    if modifiers
        != (gpui::Modifiers {
            control: true,
            ..Default::default()
        })
    {
        return None;
    }
    SCROLLS.iter().find_map(|(sequence, _, scroll)| {
        (sequence.strip_prefix("ctrl-") == Some(key)).then_some(*scroll)
    })
}
