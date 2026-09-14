use super::*;
use serde_json::json;

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn message(text: &str) -> Value {
    json!({"role":"user", "content":[
        {"type":"text", "text":text},
        {"type":"image", "data":PNG, "mimeType":"image/png"}
    ]})
}

fn pending(tracked: bool, status: &str) -> ConversationState {
    let mut state = ConversationState::default();
    let item = state.push_local_user_with_prompt_images(
        "new input".into(),
        &[PromptImage::new(PNG.into(), "image/png".into())],
        false,
    );
    state.bind_submitted_prompt_with_evidence("new-id", &item, tracked);
    state.record_prompt_delivery("new-id", &Value::Null, status);
    state
}

#[test]
fn untracked_accepted_input_uses_history_without_duplicating_its_live_row() {
    let mut state = pending(false, "accepted");
    state.record_prompt_delivery("new-id", &Value::Null, "unknown");
    state.replace_history(&[message("new input")]);
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].text, "new input");
    assert_eq!(state.items[0].images.len(), 1);
}

#[test]
fn untracked_accepted_input_survives_empty_history_with_its_attachments() {
    let mut state = pending(false, "accepted");
    state.replace_history(&[]);
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].text, "new input");
    assert_eq!(state.items[0].images.len(), 1);
    assert!(state.items[0].label.is_empty());
}

#[test]
fn unknown_input_survives_history_without_claiming_delivery_tracking() {
    for tracked in [false, true] {
        let mut state = pending(tracked, "unknown");
        state.replace_history(&[message("old input")]);
        assert_eq!(state.items.len(), 2, "tracked={tracked}");
        assert_eq!(state.items[0].text, "old input");
        assert_eq!(state.items[1].text, "new input");
        assert_eq!(state.items[1].label, "Delivery unknown");
        assert_eq!(state.items[1].images.len(), 1);
        assert!(!state.running);
    }
}

#[test]
fn correlated_accepted_input_remains_beside_unrelated_history_until_delivered() {
    let mut state = pending(true, "accepted");
    state.replace_history(&[message("old input")]);
    assert_eq!(state.items.len(), 2);
    assert_eq!(state.items[1].text, "new input");
    assert_eq!(state.items[1].images.len(), 1);
    state.record_prompt_delivery("new-id", &Value::Null, "delivered");
    state.replace_history(&[message("old input"), message("new input")]);
    assert_eq!(state.items.len(), 2);
    assert_eq!(state.items[1].text, "new input");
}

#[test]
fn restored_legacy_receipt_does_not_gain_tracking_on_a_second_history_refresh() {
    let mut fallback = message("new input");
    fallback["submissionId"] = "legacy-id".into();
    fallback["deliveryStatus"] = "accepted".into();
    fallback["deliveryTracked"] = false.into();
    let mut state = ConversationState::default();
    state.replace_history(&[fallback]);
    assert_eq!(state.items.len(), 1);
    state.replace_history(&[message("new input")]);
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].text, "new input");
    assert_eq!(state.items[0].images.len(), 1);
}
