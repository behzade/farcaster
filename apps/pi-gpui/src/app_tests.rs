use std::time::SystemTime;

use super::*;
use crate::{
    conversation::{TranscriptItem, TranscriptKind},
    sessions::UsageSummary,
};

fn item(text: &str) -> TranscriptItem {
    TranscriptItem {
        kind: TranscriptKind::Assistant,
        label: "Pi".into(),
        text: text.into(),
        streaming: false,
        is_error: false,
        tool_call_id: None,
        tool_output: String::new(),
        tool_presentation: None,
    }
}

#[test]
fn done_is_recent_only_after_an_active_status_transition() {
    assert!(!starts_recent_completion(None, "Done", false));
    assert!(!starts_recent_completion(Some("Done"), "Done", false));
    assert!(starts_recent_completion(Some("Working"), "Done", false));
    assert!(starts_recent_completion(None, "Done", true));
    assert!(!starts_recent_completion(Some("Working"), "Failed", false));
}

#[test]
fn new_sessions_are_added_by_creation_without_resorting_existing_sessions() {
    let session = |id: &str, timestamp: &str| {
        SessionSummary::from_cached(
            id.into(),
            PathBuf::from(format!("/{id}.jsonl")),
            PathBuf::from("/project"),
            id.into(),
            String::new(),
            timestamp.into(),
            None,
            SystemTime::UNIX_EPOCH,
            0,
            UsageSummary::default(),
            false,
            false,
            String::new(),
        )
    };
    let mut order = vec!["manual-b".into(), "manual-a".into()];

    assert!(add_new_sessions_to_order(
        &mut order,
        &[
            session("manual-a", "2026-01-01"),
            session("manual-b", "2026-01-02"),
            session("new-old", "2026-01-03"),
            session("new-new", "2026-01-04"),
        ],
    ));
    assert_eq!(order, vec!["new-new", "new-old", "manual-b", "manual-a"]);
}

#[test]
fn selecting_a_subagent_does_not_invalidate_the_session_rail() {
    let session = |id: &str, parent: Option<&str>| {
        SessionSummary::from_cached(
            id.into(),
            PathBuf::from(format!("/{id}.jsonl")),
            PathBuf::from("/project"),
            id.into(),
            String::new(),
            String::new(),
            parent.map(str::to_owned),
            SystemTime::UNIX_EPOCH,
            0,
            UsageSummary::default(),
            false,
            false,
            String::new(),
        )
    };
    let sessions = vec![session("root", None), session("child", Some("root"))];
    let previous = RuntimeSnapshot {
        selected_session: Some(PathBuf::from("/root.jsonl")),
        ..RuntimeSnapshot::default()
    };
    let next = RuntimeSnapshot {
        selected_session: Some(PathBuf::from("/child.jsonl")),
        ..RuntimeSnapshot::default()
    };

    assert!(!session_rail_snapshot_changed(&sessions, &previous, &next));
}

#[test]
fn transcript_only_snapshot_changes_do_not_invalidate_other_regions() {
    let previous = RuntimeSnapshot::default();
    let mut next = previous.clone();
    next.conversation.items.push(Arc::new(item("stream update")));

    assert!(!composer_snapshot_changed(&previous, &next));
    assert!(!run_panel_snapshot_changed(&previous, &next));
}

#[test]
fn composer_and_run_panel_track_their_rendered_snapshot_inputs() {
    let previous = RuntimeSnapshot::default();
    let mut composer = previous.clone();
    composer.status = "Working".into();
    let mut run_panel = previous.clone();
    run_panel.conversation.latest_cache_hit_rate = Some(0.5);

    assert!(composer_snapshot_changed(&previous, &composer));
    assert!(!run_panel_snapshot_changed(&previous, &composer));
    assert!(run_panel_snapshot_changed(&previous, &run_panel));
    assert!(!composer_snapshot_changed(&previous, &run_panel));
}

#[test]
fn manual_session_move_is_stable() {
    let mut order = vec!["a".into(), "b".into(), "c".into()];
    assert!(move_to(&mut order, "c", "a"));
    assert_eq!(order, vec!["c", "a", "b"]);
    assert!(move_to(&mut order, "c", "b"));
    assert_eq!(order, vec!["a", "b", "c"]);
    assert!(!move_to(&mut order, "c", "c"));
}

#[test]
fn fps_debug_flag_accepts_only_literal_true() {
    let enabled = |value: Option<&str>| value == Some("true");
    assert!(enabled(Some("true")));
    assert!(!enabled(Some("TRUE")));
    assert!(!enabled(Some("1")));
    assert!(!enabled(None));
}

#[test]
fn transcript_splice_keeps_unchanged_rows_out_of_the_render_path() {
    let current = vec![item("one"), item("two"), item("three")];
    assert_eq!(transcript_splice(&current, &current), None);

    let mut updated = current.clone();
    updated[1] = item("changed");
    assert_eq!(transcript_splice(&current, &updated), Some((1..2, 1)));

    let mut appended = current.clone();
    appended.push(item("four"));
    assert_eq!(transcript_splice(&current, &appended), Some((3..3, 1)));
}

#[test]
fn extension_dialog_is_parked_and_restored_with_its_session() {
    let mut visible = ExtensionUiState::default();
    visible.apply(ExtensionUiRequest::Confirm {
        id: "approval".into(),
        title: "Permission".into(),
        message: "Allow it?".into(),
        timeout: None,
    });
    let mut parked = None;

    park_extension_surface(&mut visible, &mut parked);
    assert!(visible.dialog.is_none());
    assert_eq!(
        parked
            .as_ref()
            .and_then(|session| session.dialog.as_ref())
            .and_then(ExtensionUiRequest::dialog_id),
        Some("approval")
    );
    parked
        .as_mut()
        .expect("live session surface should be parked")
        .apply(ExtensionUiRequest::Input {
            id: "follow-up".into(),
            title: "Need a note".into(),
            placeholder: None,
            timeout: None,
        });

    restore_extension_surface(&mut visible, &mut parked);
    assert!(parked.is_none());
    assert_eq!(
        visible
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id),
        Some("approval")
    );
    assert!(visible.respond_confirm("approval", true).is_some());
    assert_eq!(
        visible
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id),
        Some("follow-up")
    );
}
