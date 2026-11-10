use super::*;
use serde_json::json;

#[test]
fn compaction_updates_one_row_without_moving_interleaved_messages() {
    let mut state = ConversationState::default();
    assert_eq!(state.reduce(&json!({"type":"compaction_start"})), Some(0));
    let before = state.clone();
    state.push_local_user("Next task".into(), 0, false);
    assert_eq!(state.reduce(&json!({"type":"compaction_end"})), Some(0));
    assert_eq!(state.items.len(), 2);
    assert_eq!(
        state.items[0].kind,
        TranscriptKind::Compaction(CompactionState::Complete)
    );
    assert_eq!(state.items[0].text, "Context compacted");
    assert!(!state.items[0].streaming);
    assert!(!state.compacting);
    assert_eq!(state.items[1].text, "Next task");
    assert_eq!(
        before.items[0].kind,
        TranscriptKind::Compaction(CompactionState::Running)
    );
    assert!(before.items[0].streaming);
}

#[test]
fn compaction_keeps_failures_and_cancellations_distinct() {
    for (end, status, text) in [
        (
            json!({"errorMessage":"Provider unavailable"}),
            CompactionState::Failed,
            "Compaction failed: Provider unavailable",
        ),
        (
            json!({"aborted":true}),
            CompactionState::Aborted,
            "Compaction cancelled",
        ),
        (
            json!({"errorMessage":null}),
            CompactionState::Complete,
            "Context compacted",
        ),
        (
            json!({"errorMessage":"  "}),
            CompactionState::Complete,
            "Context compacted",
        ),
    ] {
        let mut state = ConversationState::default();
        state.reduce(&json!({"type":"compaction_start","reason":"manual"}));
        let mut end = end;
        end["type"] = json!("compaction_end");
        state.reduce(&end);
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].kind, TranscriptKind::Compaction(status));
        assert_eq!(state.items[0].text, text);
        assert_eq!(state.items[0].is_error, status == CompactionState::Failed);
        assert!(!state.items[0].streaming);
    }
}

#[test]
fn terminal_compaction_without_start_is_visible_and_new_cycles_keep_their_own_rows() {
    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"compaction_end"}));
    state.reduce(&json!({"type":"compaction_start"}));
    state.reduce(&json!({"type":"compaction_start"}));
    assert_eq!(state.items.len(), 2);
    state.reduce(&json!({"type":"compaction_end"}));
    assert_eq!(state.items.len(), 2);
    assert!(
        state
            .items
            .iter()
            .all(|item| item.kind == TranscriptKind::Compaction(CompactionState::Complete))
    );
}

#[test]
fn settlement_stops_pending_compaction_without_claiming_success_or_hiding_model_errors() {
    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"agent_start"}));
    state.push_transport_error("Connection lost".into());
    state.reduce(&json!({"type":"compaction_start"}));
    assert_eq!(state.reduce(&json!({"type":"agent_settled"})), Some(1));
    assert_eq!(
        state.items[1].kind,
        TranscriptKind::Compaction(CompactionState::Interrupted)
    );
    assert!(!state.items[1].streaming);
    assert!(!state.compacting);
    assert!(state.ended_in_error());
}
