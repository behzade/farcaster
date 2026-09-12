use super::*;
use serde_json::json;

fn delivery(id: &str, text: &str, status: &str) -> Value {
    json!({"type":"prompt_delivery", "submissionId":id, "status":status,
        "message":{"role":"user", "content":text, "queued":true, "deliveryTracked":true}})
}

#[test]
fn receipt_and_cancel_orders_keep_one_row_per_submission_not_per_text() {
    let mut state = ConversationState::default();
    state.reduce(&delivery("one", "same text", "unknown"));
    assert_eq!(state.items[0].label, "Delivery unknown");
    state.reduce(&delivery("two", "same text", "accepted"));
    state.reduce(&delivery("one", "same text", "delivered"));
    state.reduce(&delivery("one", "same text", "accepted"));
    state.reduce(&delivery("two", "same text", "unknown"));
    state.reduce(&delivery("two", "same text", "rejected"));
    assert_eq!(state.items.len(), 2);
    assert!(
        state
            .items
            .iter()
            .all(|item| item.text == "same text" && item.label.is_empty())
    );
    state.reduce(&delivery("three", "unsent", "unknown"));
    state.reduce(&delivery("three", "unsent", "rejected"));
    assert_eq!(
        state.items.len(),
        2,
        "only proven unaccepted input rolls back"
    );
}

#[test]
fn receipt_during_stream_does_not_reset_or_corrupt_the_assistant() {
    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"message_start", "message":{"role":"assistant", "content":[]}}));
    state.reduce(&json!({"type":"message_update", "assistantMessageEvent":{"type":"text_delta", "contentIndex":0, "delta":"hello "}}));
    state.reduce(&delivery("one", "next", "accepted"));
    state.reduce(&json!({"type":"message_update", "assistantMessageEvent":{"type":"text_delta", "contentIndex":0, "delta":"world"}}));
    state.reduce(&json!({"type":"message_end", "message":{"role":"assistant", "content":[{"type":"text", "text":"hello world"}]}}));
    state.reduce(&delivery("one", "next", "delivered"));
    let assistants = state
        .items
        .iter()
        .filter(|item| item.kind == TranscriptKind::Assistant)
        .collect::<Vec<_>>();
    assert_eq!(assistants.len(), 1);
    assert_eq!(assistants[0].text, "hello world");
    assert_eq!(
        state
            .items
            .iter()
            .filter(|item| item.kind == TranscriptKind::User)
            .count(),
        1
    );
}

#[test]
fn unknown_normal_input_adopts_its_bound_optimistic_row_and_keeps_images() {
    let mut state = ConversationState::default();
    let image = PromptImage::new("AQID".into(), "image/png".into());
    let item = state.push_local_user_with_prompt_images("local input".into(), &[image], false);
    state.bind_submitted_prompt("normal", &item);
    state.reduce(&json!({"type":"message_start", "message":{"role":"assistant", "content":[]}}));
    state.record_prompt_delivery("normal", &Value::Null, "unknown");
    assert_eq!(state.items[0].label, "Delivery unknown");
    assert_eq!(state.items[0].images.len(), 1);
    state.record_prompt_delivery("normal", &Value::Null, "accepted");
    assert_eq!(
        state
            .items
            .iter()
            .filter(|item| item.kind == TranscriptKind::User)
            .count(),
        1
    );
    assert!(state.items[0].label.is_empty());
}

#[test]
fn ordinary_user_echo_reconciles_a_bound_row_after_receipt_changes() {
    for status in ["accepted", "unknown"] {
        let mut state = ConversationState::default();
        let image = PromptImage::new("AQID".into(), "image/png".into());
        let original = state.push_local_user_with_prompt_images("look".into(), &[image], false);
        state.bind_submitted_prompt("normal", &original);
        state.record_prompt_delivery("normal", &Value::Null, status);
        let message = json!({"role":"user", "content":[
            {"type":"text", "text":"look"}, {"type":"image", "data":"AQID", "mimeType":"image/png"}
        ]});
        for kind in ["message_start", "message_end"] {
            state.reduce(&json!({"type":kind, "message":message}));
        }
        state.record_prompt_delivery("normal", &Value::Null, "accepted");
        assert_eq!(
            state.items.len(),
            1,
            "ordinary echo must retain the bound identity after {status}"
        );
        assert_eq!(state.items[0].images.len(), 1);
        assert!(state.items[0].label.is_empty());
    }
}

#[test]
fn rejection_before_active_mixed_content_keeps_projection_offsets_valid() {
    let mut state = ConversationState::default();
    state.reduce(&delivery("reject", "unsent", "unknown"));
    state.reduce(&json!({"type":"agent_start"}));
    state.reduce(&json!({"type":"message_start", "message":{"role":"assistant", "content":[]}}));
    state.reduce_deferred(&json!({"type":"message_update", "assistantMessageEvent":{"type":"thinking_delta", "contentIndex":0, "delta":"think "}}));
    state.reduce_deferred(&json!({"type":"message_update", "assistantMessageEvent":{"type":"text_delta", "contentIndex":1, "delta":"hello "}}));
    state.flush_live_projection();
    state.reduce(&json!({"type":"tool_execution_start", "toolCallId":"tool-1", "toolName":"read", "args":{"path":"one"}}));
    state.reduce(&delivery("reject", "unsent", "rejected"));
    state.reduce_deferred(&json!({"type":"message_update", "assistantMessageEvent":{"type":"thinking_delta", "contentIndex":0, "delta":"done"}}));
    state.reduce_deferred(&json!({"type":"message_update", "assistantMessageEvent":{"type":"text_delta", "contentIndex":1, "delta":"world"}}));
    state.flush_live_projection();
    state.reduce(&json!({"type":"tool_execution_end", "toolCallId":"tool-1", "toolName":"read", "result":{"content":[{"type":"text", "text":"file bytes"}]}}));
    assert_eq!(
        state
            .items
            .iter()
            .find(|item| item.kind == TranscriptKind::Thinking)
            .unwrap()
            .complete_text(),
        "think done"
    );
    assert_eq!(
        state
            .items
            .iter()
            .find(|item| item.kind == TranscriptKind::Assistant)
            .unwrap()
            .complete_text(),
        "hello world"
    );
    assert_eq!(
        state
            .items
            .iter()
            .filter(|item| item.kind == TranscriptKind::Tool)
            .count(),
        1
    );
    assert!(
        state
            .items
            .iter()
            .find(|item| item.kind == TranscriptKind::Tool)
            .unwrap()
            .tool_output
            .contains("file bytes")
    );
    assert_eq!(state.active_run_start(), Some(0));
}

#[test]
fn history_refresh_retains_unresolved_receipts_and_reconciles_exact_identity() {
    let mut state = ConversationState::default();
    state.reduce(&delivery("unknown", "same text", "unknown"));
    state.reduce(&delivery("accepted", "same text", "accepted"));
    state.replace_history(&[
        json!({"role":"assistant", "content":[{"type":"text", "text":"old reply"}]}),
    ]);
    assert_eq!(state.items.len(), 3);
    assert_eq!(state.items[1].label, "Delivery unknown");
    state.replace_history(&[
        json!({"role":"assistant", "content":[{"type":"text", "text":"old reply"}]}),
        json!({"role":"user", "content":"same text", "submissionId":"accepted"}),
    ]);
    assert_eq!(state.items.len(), 3);
    assert_eq!(
        state
            .items
            .iter()
            .filter(|item| item.label == "Delivery unknown")
            .count(),
        1
    );
    state.reduce(&delivery("accepted", "same text", "accepted"));
    assert_eq!(state.items.len(), 3);
}

#[test]
fn historical_submission_cannot_adopt_an_unrelated_optimistic_row() {
    let mut state = ConversationState::default();
    let image = PromptImage::new("AQID".into(), "image/png".into());
    let pending = state.push_local_user_with_prompt_images("pending text".into(), &[image], false);
    state.bind_submitted_prompt_with_evidence("pending", &pending, true);
    state.replace_history(&[
        json!({"role":"user", "content":"historical text", "submissionId":"history"}),
    ]);
    assert_eq!(state.items.len(), 2);
    assert_eq!(state.items[0].text, "historical text");
    assert!(state.items[0].images.is_empty());
    assert_eq!(state.items[1].text, "pending text");
    assert_eq!(state.items[1].images.len(), 1);
}
