use std::time::SystemTime;

use super::*;
use crate::{
    agent_activity::{AgentActivity, AgentLifecycle},
    conversation::{TranscriptItem, TranscriptKind},
    sessions::UsageSummary,
};

#[test]
fn close_targets_a_draft_before_its_backing_session() {
    let session = std::path::Path::new("/tmp/session.jsonl");

    assert_eq!(
        current_close_target(Some("draft-1"), Some(session)),
        CurrentCloseTarget::Draft("draft-1".into())
    );
    assert_eq!(
        current_close_target(None, Some(session)),
        CurrentCloseTarget::Session(session.into())
    );
}

fn item(text: &str) -> TranscriptItem {
    TranscriptItem {
        kind: TranscriptKind::Assistant,
        label: "Pi".into(),
        text: text.into(),
        stream_chunks: Arc::default(),
        streaming: false,
        is_error: false,
        tool_call_id: None,
        tool_output: String::new(),
        tool_presentation: None,
        invocation: None,
    }
}

fn session_summary(id: &str, parent: Option<&str>, settled: bool) -> SessionSummary {
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
        settled,
        false,
        String::new(),
    )
}

#[test]
fn unchanged_scroll_follow_event_does_not_invalidate_transcript() {
    let mut following = true;
    let mut unseen = 0;
    assert!(!update_transcript_follow_state(
        &mut following,
        &mut unseen,
        true,
    ));

    unseen = 3;
    assert!(update_transcript_follow_state(
        &mut following,
        &mut unseen,
        true,
    ));
    assert_eq!(unseen, 0);
    assert!(update_transcript_follow_state(
        &mut following,
        &mut unseen,
        false,
    ));
    assert!(!update_transcript_follow_state(
        &mut following,
        &mut unseen,
        false,
    ));
}

#[test]
fn unchanged_catalog_poll_does_not_invalidate_session_regions() {
    assert!(!session_catalog_changed(&[], &[], None, &[], &[]));
    assert!(session_catalog_changed(
        &[],
        &[],
        Some("previous failure"),
        &[],
        &[],
    ));
}

#[test]
fn session_catalog_changes_invalidate_only_the_regions_that_render_them() {
    let session = |modified, usage| {
        SessionSummary::from_cached(
            "root".into(),
            PathBuf::from("/root.jsonl"),
            PathBuf::from("/project"),
            "Root".into(),
            String::new(),
            "2026-01-01".into(),
            None,
            modified,
            1,
            usage,
            false,
            false,
            String::new(),
        )
    };
    let current = vec![session(SystemTime::UNIX_EPOCH, UsageSummary::default())];
    let touched = vec![session(SystemTime::now(), UsageSummary::default())];

    let selected = Some(std::path::Path::new("/root.jsonl"));
    assert!(!run_panel_sessions_changed(&current, &touched, selected));
    assert!(!composer_usage_sessions_changed(
        &current, &touched, selected,
    ));

    let changed = vec![session(
        SystemTime::now(),
        UsageSummary {
            cost_micros: 1,
            ..UsageSummary::default()
        },
    )];
    assert!(!run_panel_sessions_changed(&current, &changed, selected));
    assert!(composer_usage_sessions_changed(
        &current, &changed, selected,
    ));
}

#[test]
fn switching_between_active_sessions_does_not_invalidate_archived_sessions() {
    let sessions = vec![
        session_summary("first", None, false),
        session_summary("second", None, false),
    ];
    let previous = RuntimeSnapshot {
        selected_session: Some(PathBuf::from("/first.jsonl")),
        ..RuntimeSnapshot::default()
    };
    let next = RuntimeSnapshot {
        selected_session: Some(PathBuf::from("/second.jsonl")),
        ..RuntimeSnapshot::default()
    };

    assert!(!archived_session_rail_snapshot_changed(
        &sessions, &previous, &next,
    ));

    let archived_sessions = vec![
        session_summary("first", None, false),
        session_summary("second", None, true),
    ];
    assert!(archived_session_rail_snapshot_changed(
        &archived_sessions,
        &previous,
        &next,
    ));
}

#[test]
fn active_session_discovery_does_not_invalidate_archived_sessions() {
    let archived = session_summary("archived", None, true);
    let current = vec![archived.clone()];
    let with_active = vec![session_summary("active", None, false), archived.clone()];

    assert!(!archived_session_catalog_changed(
        &current,
        &current,
        &with_active,
        &with_active,
    ));
    let mut renamed = archived;
    renamed.title = "Renamed".into();
    assert!(archived_session_catalog_changed(
        &current,
        &current,
        &[renamed],
        &current,
    ));
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
fn selecting_a_subagent_does_not_invalidate_the_session_rail() {
    let sessions = vec![
        session_summary("root", None, false),
        session_summary("child", Some("root"), false),
    ];
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

fn activity(id: &str, text: &str) -> AgentActivity {
    AgentActivity {
        session_id: id.into(),
        session_path: PathBuf::from(format!("/{id}.jsonl")),
        role: String::new(),
        activity: text.into(),
        lifecycle: AgentLifecycle::Working,
        current_tool: None,
        recent_tool: None,
        tool_call_count: 0,
        limited: false,
        usage: UsageSummary::default(),
        started: SystemTime::UNIX_EPOCH,
        ended: None,
        elapsed: None,
        changed_paths: Vec::new(),
        file_mutations: Vec::new(),
    }
}

#[test]
fn run_panel_ignores_activity_changes_outside_the_selected_tree() {
    let sessions = vec![
        session_summary("root", None, false),
        session_summary("child", Some("root"), false),
        session_summary("other", None, false),
    ];
    let current = HashMap::from([
        ("child".into(), activity("child", "working")),
        ("other".into(), activity("other", "working")),
    ]);
    let unrelated = HashMap::from([("other".into(), activity("other", "changed"))]);
    let visible = HashMap::from([("child".into(), activity("child", "changed"))]);

    assert!(!run_panel_activities_changed(
        &current,
        Some(&(unrelated, false)),
        &sessions,
        Some(std::path::Path::new("/root.jsonl")),
    ));
    assert!(run_panel_activities_changed(
        &current,
        Some(&(visible, false)),
        &sessions,
        Some(std::path::Path::new("/root.jsonl")),
    ));
    assert!(run_panel_activities_changed(
        &current,
        Some(&(HashMap::new(), true)),
        &sessions,
        Some(std::path::Path::new("/root.jsonl")),
    ));
}

#[test]
fn transcript_only_snapshot_changes_do_not_invalidate_other_regions() {
    let mut previous = RuntimeSnapshot::default();
    Arc::make_mut(&mut previous.conversation)
        .items
        .push(Arc::new(item("existing")));
    let mut next = previous.clone();
    Arc::make_mut(&mut next.conversation)
        .items
        .push(Arc::new(item("stream update")));

    assert!(!composer_snapshot_changed(&previous, &next));
    assert!(!run_panel_snapshot_changed(&previous, &next));
}

#[test]
fn composer_variant_tracks_empty_to_nonempty_conversations() {
    let previous = RuntimeSnapshot::default();
    let mut connected = previous.clone();
    connected.connected = true;
    assert!(!composer_snapshot_changed(&previous, &connected));

    let mut conversation = previous.clone();
    Arc::make_mut(&mut conversation.conversation)
        .items
        .push(Arc::new(item("first response")));
    assert!(composer_snapshot_changed(&previous, &conversation));
}

#[test]
fn restored_questions_invalidate_the_composer() {
    let previous = RuntimeSnapshot::default();
    let next = RuntimeSnapshot {
        pending_question: Some(ExtensionUiRequest::Input {
            id: "restored-question:one".into(),
            title: "Which command?".into(),
            placeholder: None,
            timeout: None,
        }),
        ..previous.clone()
    };

    assert!(composer_snapshot_changed(&previous, &next));
}

#[test]
fn composer_and_run_panel_track_their_rendered_snapshot_inputs() {
    let previous = RuntimeSnapshot::default();
    let mut composer_status = previous.clone();
    composer_status.status = "Working".into();
    let mut composer_usage = previous.clone();
    Arc::make_mut(&mut composer_usage.conversation).average_cache_hit_rate = Some(0.5);
    let mut composer_context = previous.clone();
    composer_context.stats = serde_json::json!({
        "contextUsage": {"tokens": 10_000, "contextWindow": 200_000}
    });
    let mut run_panel = previous.clone();
    run_panel.selected_session = Some(PathBuf::from("/root.jsonl"));

    assert!(!composer_snapshot_changed(&previous, &composer_status));
    assert!(!run_panel_snapshot_changed(&previous, &composer_status));
    assert!(composer_snapshot_changed(&previous, &composer_usage));
    assert!(!run_panel_snapshot_changed(&previous, &composer_usage));
    assert!(composer_snapshot_changed(&previous, &composer_context));
    assert!(!run_panel_snapshot_changed(&previous, &composer_context));
    assert!(composer_snapshot_changed(&previous, &run_panel));
    assert!(run_panel_snapshot_changed(&previous, &run_panel));
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
