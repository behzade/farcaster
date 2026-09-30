use super::{ComposerSessions, ComposerSnapshot, HistoryNavigation};

#[test]
fn composer_saves_share_history_and_keep_queued_snapshots_unchanged() {
    use super::{ComposerPersistence, ComposerRecord};
    use std::{cell::RefCell, future::Future, pin::Pin, rc::Rc, sync::Arc};

    struct RecordingPersistence(Rc<RefCell<Vec<ComposerRecord<String>>>>);

    impl ComposerPersistence<String> for RecordingPersistence {
        fn save(&self, record: ComposerRecord<String>) {
            self.0.borrow_mut().push(record);
        }

        fn delete(&self, _target: String) {}

        fn flush(&self) -> Pin<Box<dyn Future<Output = Result<(), String>>>> {
            Box::pin(async { Ok(()) })
        }

        fn revision(&self) -> u64 {
            self.0.borrow().len() as u64
        }
    }

    let records = Rc::new(RefCell::new(Vec::new()));
    let history: Arc<Vec<String>> =
        Arc::new((0..100).map(|index| format!("entry-{index}")).collect());
    let mut sessions = ComposerSessions::new(
        "draft:one".into(),
        vec![ComposerRecord {
            target: "draft:one".into(),
            history: history.clone(),
            ..Default::default()
        }],
        Box::new(RecordingPersistence(records.clone())),
    );
    sessions.capture_current(ComposerSnapshot::new("draft".into(), 5, 5..5));
    sessions.capture_current(ComposerSnapshot::new("draft!".into(), 6, 6..6));
    sessions.capture_current(ComposerSnapshot::new("draft!".into(), 1, 0..1));
    sessions.capture_current(ComposerSnapshot::new("draft!".into(), 1, 0..1));
    assert_eq!(sessions.persistence_revision(), 3);
    assert!(
        records
            .borrow()
            .iter()
            .all(|record| Arc::ptr_eq(&record.history, &history))
    );

    sessions.record_submission("draft:one", "  newest  ");
    assert_eq!(sessions.sessions["draft:one"].history.len(), 100);
    assert_eq!(sessions.sessions["draft:one"].history[0], "newest");
    assert_eq!(sessions.sessions["draft:one"].history[99], "entry-98");
    sessions.sync_history(
        "draft:one",
        ["replacement", "replacement", "second"].into_iter(),
    );
    let replacement = sessions.sessions["draft:one"].history.clone();
    sessions.promote("draft:one", "session:one".into());

    let records = records.borrow();
    assert_eq!(records.len(), 6);
    assert_eq!(records[0].text, "draft");
    assert_eq!(records[1].text, "draft!");
    assert_eq!(records[2].cursor, 1);
    assert_eq!(
        (records[2].selection_start, records[2].selection_end),
        (0, 1)
    );
    assert_eq!(records[0].history[0], "entry-0");
    assert_eq!(records[0].history[99], "entry-99");
    assert_eq!(records[3].history[0], "newest");
    assert_eq!(records[3].history[99], "entry-98");
    assert_eq!(records[4].history.as_slice(), ["replacement", "second"]);
    assert!(Arc::ptr_eq(&records[4].history, &records[5].history));
    assert!(Arc::ptr_eq(&records[5].history, &replacement));
    assert_eq!(records[5].target, "session:one");
}

fn sessions(target: &str) -> ComposerSessions<String> {
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
fn failed_send_to_chat_keeps_both_drafts_and_destination_cursor() {
    let mut sessions = sessions("session:destination");
    let destination = ComposerSnapshot::new("existing draft".into(), 3, 1..3);
    sessions.switch_to("session:source".into(), destination.clone());
    let attachment = "/project/notes.txt".to_owned();
    sessions.set_attachments("session:destination", vec![attachment.clone()]);
    let source = ComposerSnapshot::new("source draft".into(), 2, 2..2);
    sessions.capture_current(source.clone());
    sessions.record_submission("session:destination", "message");
    assert_eq!(sessions.snapshot_for("session:destination"), destination);
    let recovered = sessions.append_to_draft("session:destination", "message");
    assert_eq!(recovered.text, "existing draft\n\nmessage");
    assert_eq!(recovered.cursor, destination.cursor);
    assert_eq!(recovered.selection, destination.selection);
    assert_eq!(sessions.snapshot_for("session:source"), source);
    assert_eq!(
        sessions.saved_attachments().next().map(|(_, files)| files),
        Some([attachment].as_slice())
    );
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
fn synced_history_trims_deduplicates_and_bounds_the_latest_messages() {
    let mut sessions = sessions("session:one");
    let mut messages = (0..105).map(|index| index.to_string()).collect::<Vec<_>>();
    messages.extend([" 104 ".into(), " ".into(), " 105 ".into(), "105".into()]);
    sessions.sync_history("session:one", messages.iter().rev().map(String::as_str));
    let history = &sessions.sessions["session:one"].history;
    assert_eq!(history.len(), 100);
    assert_eq!(history.first().map(String::as_str), Some("105"));
    assert_eq!(history.last().map(String::as_str), Some("6"));

    let first = history[0].as_ptr();
    let draft = ComposerSnapshot::new("unsent".into(), 6, 6..6);
    let _ = sessions.previous_history(draft.clone());
    sessions.sync_history("session:one", messages.iter().rev().map(String::as_str));
    assert_eq!(sessions.sessions["session:one"].history[0].as_ptr(), first);
    assert_eq!(sessions.sessions["session:one"].history_index, Some(0));
    assert_eq!(sessions.sessions["session:one"].history_draft, Some(draft));

    sessions.sync_history("session:one", std::iter::empty());
    assert_eq!(sessions.sessions["session:one"].history.len(), 100);
    sessions.sync_history("session:one", [" ", "\n"].into_iter());
    assert!(sessions.sessions["session:one"].history.is_empty());
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

#[test]
fn empty_composer_state_survives_switching_between_drafts() {
    let mut sessions = sessions("draft:one");
    sessions.capture_current(ComposerSnapshot::new("text".into(), 4, 4..4));
    sessions.capture_current(ComposerSnapshot::default());

    sessions.switch_to("draft:two".into(), ComposerSnapshot::default());
    sessions.switch_to("draft:one".into(), ComposerSnapshot::default());
    assert_eq!(sessions.current(), ComposerSnapshot::default());
    assert!(sessions.clear_submitted_text("draft:one", ""));
}
