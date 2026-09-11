use super::*;

fn activated(state: &mut Activation, key: &str, now: Instant) -> ActivatedKey {
    let stroke = gpui::Keystroke::parse(key).expect("test operation should succeed");
    state.key(&stroke.key, stroke.modifiers, now)
}

#[test]
fn control_editing_keys_pass_through_without_leader() {
    let now = Instant::now();
    for key in ["ctrl-f", "ctrl-b", "ctrl-u", "ctrl-d"] {
        assert_eq!(
            activated(&mut Activation::default(), key, now),
            ActivatedKey::Pass
        );
    }
}

#[test]
fn activation_is_not_a_leader_and_only_double_g_returns() {
    let now = Instant::now();
    let mut state = Activation::default();
    assert_eq!(activated(&mut state, "ctrl-g", now), ActivatedKey::Pending);
    assert_eq!(
        activated(&mut state, "2", now),
        ActivatedKey::Command(Command::Session(2))
    );
    assert_eq!(activated(&mut state, "2", now), ActivatedKey::Pass);
    assert_eq!(activated(&mut state, "ctrl-g", now), ActivatedKey::Pending);
    assert_eq!(
        activated(&mut state, "e", now),
        ActivatedKey::Command(Command::Editor)
    );
    assert_eq!(activated(&mut state, "ctrl-g", now), ActivatedKey::Pending);
    assert_eq!(activated(&mut state, "ctrl-g", now), ActivatedKey::Return);
    assert!(state.deadline.is_none());
}

#[test]
fn activation_routes_commands_without_space() {
    let now = Instant::now();
    for (key, command) in [
        ("j", Command::RelativeSession(1)),
        ("k", Command::RelativeSession(-1)),
        ("q", Command::Quit),
    ] {
        let mut state = Activation::default();
        assert_eq!(activated(&mut state, key, now), ActivatedKey::Pass);
        assert_eq!(activated(&mut state, "ctrl-g", now), ActivatedKey::Pending);
        assert_eq!(
            activated(&mut state, key, now + Duration::from_millis(1900)),
            ActivatedKey::Command(command)
        );
        assert_eq!(
            activated(&mut state, key, now + Duration::from_millis(1950)),
            ActivatedKey::Pass
        );
    }
}

#[test]
fn timeout_cancellation_and_modified_keys_do_not_leak() {
    let now = Instant::now();
    let mut state = Activation::default();
    activated(&mut state, "ctrl-g", now);
    assert_eq!(
        activated(&mut state, "2", now + ACTIVATION_TIMEOUT),
        ActivatedKey::Pass
    );
    activated(&mut state, "ctrl-g", now);
    assert_eq!(
        activated(&mut state, "ctrl-g", now + ACTIVATION_TIMEOUT),
        ActivatedKey::Pending
    );
    for key in ["escape", "ctrl-2", "z", "ctrl-shift-n", "alt-shift-n"] {
        state.clear();
        activated(&mut state, "ctrl-g", now);
        assert_eq!(activated(&mut state, key, now), ActivatedKey::Cancel);
        assert_eq!(activated(&mut state, "i", now), ActivatedKey::Pass);
    }
    activated(&mut state, "ctrl-g", now);
    assert_eq!(
        activated(&mut state, "ctrl-f", now),
        ActivatedKey::Scroll(Scroll::Pages(1.0))
    );
    activated(&mut state, "ctrl-g", now);
    state.clear();
    assert_eq!(activated(&mut state, "2", now), ActivatedKey::Pass);
}

#[test]
fn activation_routes_bare_surfaces_and_transcript_boundaries() {
    let now = Instant::now();
    let mut state = Activation::default();
    for (key, command) in [
        ("space", Command::Actions),
        ("e", Command::Editor),
        ("v", Command::TranscriptScratch),
        ("c", Command::CommentCode),
        ("n", Command::NewSession),
        ("shift-n", Command::StartCodeTask),
        ("t", Command::Terminal),
        ("p", Command::AddProject),
        ("s", Command::Sandbox),
        ("m", Command::Runtime),
        ("h", Command::Harness),
        ("a", Command::RestoreSession),
    ] {
        activated(&mut state, "ctrl-g", now);
        assert_eq!(
            activated(&mut state, key, now),
            ActivatedKey::Command(command)
        );
        assert_eq!(activated(&mut state, key, now), ActivatedKey::Pass);
    }
    activated(&mut state, "ctrl-g", now);
    assert_eq!(activated(&mut state, "g", now), ActivatedKey::Pending);
    assert_eq!(
        activated(&mut state, "g", now),
        ActivatedKey::Scroll(Scroll::Start)
    );
    activated(&mut state, "ctrl-g", now);
    assert_eq!(
        activated(&mut state, "G", now),
        ActivatedKey::Scroll(Scroll::End)
    );
    for key in ["e", "escape"] {
        activated(&mut state, "ctrl-g", now);
        activated(&mut state, "g", now);
        assert_eq!(activated(&mut state, key, now), ActivatedKey::Cancel);
        assert_eq!(activated(&mut state, "g", now), ActivatedKey::Pass);
    }
    activated(&mut state, "ctrl-g", now);
    activated(&mut state, "g", now);
    assert_eq!(
        activated(&mut state, "g", now + ACTIVATION_TIMEOUT),
        ActivatedKey::Pass
    );
    assert_eq!(shortcuts::activated_command("e", Some(Prefix::G)), None);
}

#[test]
fn scrolling_respects_leader_and_exact_modifiers() {
    for (key, prefix, expected) in [
        ("g", None, None),
        ("g", Some(Prefix::G), Some(Scroll::Start)),
        ("G", None, Some(Scroll::End)),
        ("G", Some(Prefix::G), Some(Scroll::End)),
        ("ctrl-g", Some(Prefix::G), None),
        ("alt-g", Some(Prefix::G), None),
        ("ctrl-shift-g", None, None),
        ("j", None, None),
        ("k", None, None),
        ("ctrl-f", None, Some(Scroll::Pages(1.0))),
        ("ctrl-b", None, Some(Scroll::Pages(-1.0))),
        ("ctrl-d", None, Some(Scroll::Pages(0.5))),
        ("ctrl-u", None, Some(Scroll::Pages(-0.5))),
        ("ctrl-j", None, None),
        ("f", None, None),
        ("ctrl-shift-f", None, None),
        ("cmd-f", None, None),
    ] {
        let stroke = gpui::Keystroke::parse(key).expect("test keystroke");
        assert_eq!(
            transcript_scroll(&stroke.key, stroke.modifiers, prefix),
            expected,
            "{key}, prefix={prefix:?}"
        );
    }
}

#[test]
fn prefix_chord_is_ctrl_g_only() {
    let ctrl = gpui::Modifiers {
        control: true,
        ..Default::default()
    };
    let cmd = gpui::Modifiers {
        platform: true,
        ..Default::default()
    };
    assert!(is_prefix_chord("g", ctrl));
    assert!(!is_prefix_chord("g", cmd));
    assert!(!is_prefix_chord("c", ctrl));
    assert!(!is_prefix_chord(
        "g",
        gpui::Modifiers {
            shift: true,
            ..ctrl
        },
    ));
    assert!(!is_prefix_chord("g", gpui::Modifiers { alt: true, ..ctrl },));
}

#[test]
fn help_lists_ctrl_g_prefix_and_direct_composer_return() {
    let rows = shortcuts::help_shortcuts();
    for key in ["ctrl-f", "ctrl-b", "ctrl-u", "ctrl-d", "q"] {
        assert!(
            rows.iter()
                .any(|(section, chord, _)| *section == "From anywhere"
                    && chord == &format!("ctrl-g {key}"))
        );
    }
    assert!(
        rows.iter()
            .any(|(section, key, label)| *section == "From anywhere"
                && key == "ctrl-g"
                && label.contains("no focus change"))
    );
    assert!(
        rows.iter()
            .any(|(section, key, label)| *section == "From anywhere"
                && key == "ctrl-g ctrl-g"
                && *label == "Return to chat composer")
    );
}
