use super::*;
use serde_json::json;

#[test]
fn routine_compaction_only_changes_activity() {
    let mut state = ConversationState::default();
    let start = json!({"type":"compaction_start"});
    let end = json!({"type":"compaction_end"});
    for error in [json!(null), json!("  ")] {
        let end = json!({"type":"compaction_end", "errorMessage":error});
        assert_eq!(state.reduce_deferred_with_change(&end), (None, false));
    }
    assert_eq!(state.reduce_deferred_with_change(&start), (None, true));
    assert_eq!(state.reduce_deferred_with_change(&start), (None, false));
    assert!(state.compacting);
    assert!(state.items.is_empty());
    assert_eq!(state.reduce_deferred_with_change(&end), (None, true));
    assert!(!state.compacting);
    assert!(state.items.is_empty());
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
