use crate::app::composer_sessions::{ComposerSessions, ComposerSnapshot, HistoryNavigation};

fn sessions(target: &str) -> ComposerSessions {
    ComposerSessions::for_test(target.into())
}

#[test]
fn switch_restores_text_cursor_and_selection_per_session() {
    let mut sessions = sessions("draft:one");
    let first = ComposerSnapshot::new("first draft".into(), 5, 1..5);
    assert_eq!(
        sessions.switch_to("session:two".into(), first.clone()),
        ComposerSnapshot::default()
    );
    let second = ComposerSnapshot::new("second".into(), 2, 2..2);
    assert_eq!(sessions.switch_to("draft:one".into(), second), first);
}

#[test]
fn history_cycles_and_restores_the_unsent_draft() {
    let mut sessions = sessions("session:one");
    sessions.record_submission("session:one", "old");
    sessions.record_submission("session:one", "new");
    let draft = ComposerSnapshot::new("unsent".into(), 3, 3..3);

    assert_eq!(
        sessions
            .previous_history(draft.clone())
            .map(|item| item.text),
        Some("new".into())
    );
    assert_eq!(
        sessions
            .previous_history(draft.clone())
            .map(|item| item.text),
        Some("old".into())
    );
    assert!(sessions.previous_history(draft.clone()).is_none());
    assert_eq!(
        sessions.navigate_history("down", draft.clone()),
        HistoryNavigation::Handled(Some(ComposerSnapshot::new("new".into(), 3, 3..3)))
    );
    assert_eq!(
        sessions.navigate_history("down", draft.clone()),
        HistoryNavigation::Handled(Some(draft))
    );
    assert_eq!(
        sessions.navigate_history("down", ComposerSnapshot::default()),
        HistoryNavigation::PassThrough
    );
}

#[test]
fn history_keys_only_take_over_at_the_text_edges() {
    let mut sessions = sessions("session:one");
    sessions.record_submission("session:one", "sent");

    let middle = ComposerSnapshot::new("top\nbottom".into(), 6, 6..6);
    assert_eq!(
        sessions.navigate_history("up", middle.clone()),
        HistoryNavigation::PassThrough
    );
    let top = ComposerSnapshot::new("top\nbottom".into(), 0, 0..0);
    assert!(matches!(
        sessions.navigate_history("up", top),
        HistoryNavigation::Handled(Some(_))
    ));

    sessions.exit_history();
    let single_line_end = ComposerSnapshot::new("draft".into(), 5, 5..5);
    assert!(matches!(
        sessions.navigate_history("up", single_line_end),
        HistoryNavigation::Handled(Some(_))
    ));

    let first_line = ComposerSnapshot::new("top\nbottom".into(), 3, 3..3);
    assert_eq!(
        sessions.navigate_history("down", first_line),
        HistoryNavigation::PassThrough
    );
    let bottom = ComposerSnapshot::new("sent".into(), 4, 4..4);
    assert!(matches!(
        sessions.navigate_history("down", bottom),
        HistoryNavigation::Handled(Some(_))
    ));
}

#[test]
fn reversed_selection_restores_its_cursor_side() {
    let snapshot = ComposerSnapshot::new("abcdef".into(), 2, 2..5);
    let restored = snapshot.restore_range();
    assert_eq!(restored.start, 5);
    assert_eq!(restored.end, 2);
}

#[test]
fn an_accepted_prompt_only_clears_its_unchanged_session() {
    let mut sessions = sessions("session:one");
    let sent = ComposerSnapshot::new("sent".into(), 4, 4..4);
    sessions.capture_current(sent.clone());
    sessions.switch_to("session:two".into(), sent);
    sessions.capture_current(ComposerSnapshot::new("other".into(), 2, 2..2));

    assert!(sessions.clear_submitted_text("session:one", "sent"));
    assert_eq!(
        sessions.snapshot_for("session:one"),
        ComposerSnapshot::default()
    );
    assert_eq!(sessions.current().text, "other");

    sessions.capture_current(ComposerSnapshot::new("edited".into(), 6, 6..6));
    assert!(!sessions.clear_submitted_text("session:two", "other"));
    assert_eq!(sessions.current().text, "edited");
}

#[test]
fn rejected_submission_restores_only_an_empty_composer() {
    let mut sessions = sessions("session:one");
    sessions.capture_current(ComposerSnapshot::new("sent".into(), 4, 4..4));
    assert!(sessions.clear_submitted_text("session:one", "sent"));

    assert_eq!(
        sessions
            .restore_submitted_text("session:one", "sent".into())
            .map(|snapshot| snapshot.text),
        Some("sent".into())
    );
    sessions.capture_current(ComposerSnapshot::new("new text".into(), 8, 8..8));
    assert!(
        sessions
            .restore_submitted_text("session:one", "sent".into())
            .is_none()
    );
    assert_eq!(sessions.current().text, "new text");
}

#[test]
fn saving_a_draft_promotes_its_full_composer_state() {
    let mut sessions = sessions("draft:one");
    sessions.record_submission("draft:one", "first prompt");
    sessions.capture_current(ComposerSnapshot::new("next prompt".into(), 4, 1..4));

    sessions.promote("draft:one", "session:one".into());

    assert_eq!(sessions.current_target(), "session:one");
    assert_eq!(
        sessions.current(),
        ComposerSnapshot::new("next prompt".into(), 4, 1..4)
    );
    assert!(sessions.clear_submitted_text("session:one", "next prompt"));
    assert!(matches!(
        sessions.navigate_history("up", ComposerSnapshot::default()),
        HistoryNavigation::Handled(Some(ComposerSnapshot { text, .. })) if text == "first prompt"
    ));
}
