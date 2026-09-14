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
    assert!(
        state.items.is_empty(),
        "unknown queue input is not delivered"
    );
    state.reduce(&delivery("two", "same text", "accepted"));
    assert!(state.items.is_empty(), "receipt is not model delivery");
    state.reduce(&delivery("one", "same text", "delivered"));
    state.reduce(&delivery("one", "same text", "accepted"));
    state.reduce(&delivery("two", "same text", "unknown"));
    state.reduce(&delivery("two", "same text", "rejected"));
    assert_eq!(state.items.len(), 1);
    state.reduce(&delivery("two", "same text", "delivered"));
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
    let mut normal = delivery("reject", "unsent", "unknown");
    normal["message"]["queued"] = false.into();
    state.reduce(&normal);
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
            .expect("thinking item")
            .complete_text(),
        "think done"
    );
    assert_eq!(
        state
            .items
            .iter()
            .find(|item| item.kind == TranscriptKind::Assistant)
            .expect("assistant item")
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
            .expect("tool item")
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
    assert_eq!(
        state.items.len(),
        1,
        "neither queued receipt proves delivery"
    );
    assert_eq!(
        state.submitted_users.len(),
        2,
        "retain both payload identities"
    );
    state.replace_history(&[
        json!({"role":"assistant", "content":[{"type":"text", "text":"old reply"}]}),
        json!({"role":"user", "content":"same text", "submissionId":"accepted"}),
    ]);
    assert_eq!(state.items.len(), 2);
    assert_eq!(
        state
            .items
            .iter()
            .filter(|item| item.label == "Delivery unknown")
            .count(),
        0
    );
    state.reduce(&delivery("accepted", "same text", "accepted"));
    assert_eq!(state.items.len(), 2);
    state.reduce(&delivery("unknown", "same text", "delivered"));
    assert_eq!(
        state.items.len(),
        3,
        "late delivery reveals the other input once"
    );
}

#[test]
fn admitted_queue_retains_exact_images_off_transcript_across_history_refresh() {
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
    let mut state = ConversationState::default();
    let mut receipt = delivery("queued-image", "", "accepted");
    receipt["message"]["content"] = json!([
        {"type":"image", "data":PNG, "mimeType":"image/png"}
    ]);
    state.reduce(&receipt);
    assert!(state.items.is_empty());
    for history in [
        vec![],
        vec![json!({"role":"assistant", "content":[{"type":"text", "text":"old answer"}]})],
    ] {
        let expected = history.len();
        state.replace_history(&history);
        assert_eq!(state.items.len(), expected);
        assert_eq!(state.submitted_users["queued-image"].item.images.len(), 1);
    }
    receipt["status"] = "delivered".into();
    state.reduce(&receipt);
    state.reduce(&receipt);
    let users = state
        .items
        .iter()
        .filter(|item| item.kind == TranscriptKind::User)
        .collect::<Vec<_>>();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].images.len(), 1);
    assert!(users[0].text.is_empty());
}

#[test]
fn native_history_delivery_by_id_keeps_the_submitted_attachment() {
    let mut state = ConversationState::default();
    let mut accepted = delivery("image-id", "look", "accepted");
    accepted["message"]["content"] = json!([
        {"type":"text", "text":"look"},
        {"type":"image", "data":"AQID", "mimeType":"image/png"}
    ]);
    state.reduce(&accepted);
    assert!(state.items.is_empty());
    state.replace_history(&[json!({"role":"user", "submissionId":"image-id", "content":"look"})]);
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].images.len(), 1);
    state.reduce(&delivery("image-id", "look", "delivered"));
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].images.len(), 1);
    state.replace_history(&[json!({"role":"user", "submissionId":"image-id", "content":"look"})]);
    assert_eq!(state.items.len(), 1);
    assert_eq!(
        state.items[0].images.len(),
        1,
        "repeated refresh must retain exact-ID attachments"
    );
    state.replace_history(&[]);
    assert!(
        state.items.is_empty(),
        "delivered input absent from history is not appended"
    );
}

#[test]
fn unknown_image_only_queue_keeps_a_recovery_presentation_without_a_user_row() {
    let mut state = ConversationState::default();
    let mut unknown = delivery("unknown-image", "", "unknown");
    unknown["message"]["promptMode"] = "follow_up".into();
    unknown["message"]["content"] =
        json!([{ "type":"image", "data":"AQID", "mimeType":"image/png" }]);
    state.reduce(&unknown);
    state.replace_history(&[]);
    assert!(state.items.is_empty());
    let pending = state.pending_receipts();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "unknown-image");
    assert_eq!(pending[0].mode, Some(crate::protocol::PromptMode::FollowUp));
    assert!(pending[0].unknown);
    assert_eq!(pending[0].images.len(), 1);
    assert!(pending[0].text.is_empty());
    assert!(state.queue.follow_up.is_empty());
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
