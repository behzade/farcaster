use super::*;
use crate::agents::Backend;
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

#[test]
fn reopened_accepted_queue_receipts_stay_off_transcript_until_delivery()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::app::infrastructure::persistence::StateStore;
    use crate::protocol::PromptMode;
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("native-session");
    let image = PromptImage::new(PNG.into(), "image/png".into());
    let mut store = StateStore::open_at(&database)?;
    for (id, mode) in [
        ("steer", PromptMode::Steer),
        ("follow", PromptMode::FollowUp),
    ] {
        let row = store.enqueue_prompt(
            "draft:queue",
            Backend::Codex,
            temp.path(),
            None,
            mode,
            "same text",
            &[image.clone()],
        )?;
        store.complete_prompt_with_receipt(row, "draft:queue", Some(&session), id, true)?;
    }
    drop(store);
    let store = StateStore::open_at(&database)?;
    assert!(
        store.queued_prompts()?.is_empty(),
        "accepted input must never replay"
    );
    let receipts = store.accepted_prompt_history(&session)?;
    assert_eq!(receipts.len(), 2);
    for receipt in &receipts {
        assert_eq!(receipt["queued"], true);
        assert_eq!(receipt["content"][1]["data"], PNG);
    }
    let mut state = ConversationState::default();
    state.replace_history(&receipts);
    assert!(
        state.items.is_empty(),
        "reopen cannot turn receipt into delivery"
    );
    for receipt in &receipts {
        let id = receipt["submissionId"].as_str().unwrap();
        state.record_prompt_delivery(id, receipt, "delivered");
        state.record_prompt_delivery(id, receipt, "delivered");
    }
    assert_eq!(state.items.len(), 2);
    assert!(
        state
            .items
            .iter()
            .all(|item| item.text == "same text" && item.images.len() == 1)
    );
    Ok(())
}
