use super::*;
use crate::agents::Backend;

const ONE_PIXEL_PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

#[test]
fn accepted_image_only_prompt_survives_empty_backend_history_and_reopen()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session");
    let image = crate::protocol::PromptImage::new(ONE_PIXEL_PNG.into(), "image/png".into());
    let mut store = StateStore::open_at(&database)?;
    let id = store.enqueue_prompt(
        "draft:image",
        Backend::Pi,
        temp.path(),
        None,
        crate::protocol::PromptMode::Normal,
        "",
        &[image.clone()],
    )?;
    store.complete_prompt(id, "draft:image", Some(&session))?;
    store.complete_prompt(id, "draft:image", Some(&session))?;
    drop(store);

    let store = StateStore::open_at(&database)?;
    // Retain a receipt without silently resending an interrupted prompt.
    assert!(store.queued_prompts()?.is_empty());
    let mut messages = Vec::new();
    annotate_history_presentations(Some(&store), &session, &mut messages);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["submissionId"], format!("outbox:{id}"));
    assert_eq!(messages[0]["deliveryStatus"], "accepted");
    assert_eq!(
        messages[0]["content"][1],
        serde_json::json!({"type":"image", "data":image.data, "mimeType":image.mime_type})
    );
    let mut conversation = crate::conversation::ConversationState::default();
    conversation.replace_history(&messages);
    assert_eq!(conversation.items.len(), 1);
    assert_eq!(conversation.items[0].text, "");
    assert_eq!(conversation.items[0].images.len(), 1);
    let mut backend = vec![serde_json::json!({
        "role":"assistant",
        "content":[{"type":"text", "text":"existing"}],
    })];
    annotate_history_presentations(Some(&store), &session, &mut backend);
    assert_eq!(backend.len(), 1);
    assert!(
        store
            .accepted_prompt_history(&temp.path().join("other"))?
            .is_empty()
    );
    Ok(())
}

#[test]
fn uncorrelated_normal_is_not_duplicated_and_correlated_normal_survives_old_history()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session");
    let mut store = StateStore::open_at(&database)?;
    let first = store.enqueue_prompt(
        "draft:first",
        Backend::Codex,
        temp.path(),
        None,
        crate::protocol::PromptMode::Normal,
        "first",
        &[],
    )?;
    let second = store.enqueue_prompt(
        "draft:second",
        Backend::Codex,
        temp.path(),
        None,
        crate::protocol::PromptMode::Normal,
        "second",
        &[crate::protocol::PromptImage::new(
            ONE_PIXEL_PNG.into(),
            "image/png".into(),
        )],
    )?;
    store.complete_prompt_with_receipt(
        first,
        "draft:first",
        Some(&session),
        "receipt:first",
        false,
    )?;
    store.complete_prompt_with_receipt(
        second,
        "draft:second",
        Some(&session),
        "receipt:second",
        true,
    )?;
    let delivered = store.enqueue_prompt(
        "draft:delivered",
        Backend::Codex,
        temp.path(),
        None,
        crate::protocol::PromptMode::Normal,
        "already in history",
        &[],
    )?;
    store.record_prompt_receipt_delivered("receipt:delivered", Some(delivered))?;
    store.complete_prompt_with_receipt(
        delivered,
        "draft:delivered",
        Some(&session),
        "receipt:delivered",
        true,
    )?;
    drop(store);

    let store = StateStore::open_at(&database)?;
    let mut history = vec![serde_json::json!({
        "role":"user",
        "content":[{"type":"text", "text":"first"}],
    })];
    assert_eq!(
        store.accepted_prompt_history(&session)?.len(),
        2,
        "only the receipt with actual delivery evidence is excluded at storage"
    );
    annotate_history_presentations(Some(&store), &session, &mut history);
    assert_eq!(history.len(), 2);
    assert!(history[0].get("submissionId").is_none());
    assert_eq!(history[1]["submissionId"], "receipt:second");
    assert_eq!(history[1]["content"][0]["text"], "second");
    assert_eq!(
        history[1]["content"][1],
        serde_json::json!({"type":"image", "data":ONE_PIXEL_PNG, "mimeType":"image/png"})
    );
    let mut conversation = crate::conversation::ConversationState::default();
    conversation.replace_history(&history);
    assert_eq!(conversation.items.len(), 2);
    assert_eq!(conversation.items[1].text, "second");
    assert_eq!(conversation.items[1].images.len(), 1);
    Ok(())
}

#[test]
fn cold_history_restores_queued_receipt_identity_without_claiming_delivery()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::protocol::{PromptImage, PromptMode};
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session");
    let mut store = StateStore::open_at(&database)?;
    for (id, mode, tracked) in [
        ("queued-steer", PromptMode::Steer, true),
        ("queued-follow", PromptMode::FollowUp, false),
    ] {
        let row = store.enqueue_prompt(
            "draft:pending",
            Backend::Codex,
            temp.path(),
            None,
            mode,
            "same text",
            &[PromptImage::new(ONE_PIXEL_PNG.into(), "image/png".into())],
        )?;
        store.complete_prompt_with_receipt(row, "draft:pending", Some(&session), id, tracked)?;
    }
    drop(store);
    let store = StateStore::open_at(&database)?;
    let mut history =
        vec![json!({"role":"assistant", "content":[{"type":"text", "text":"older answer"}]})];
    annotate_history_presentations(Some(&store), &session, &mut history);
    assert_eq!(
        history.len(),
        3,
        "both tracked and untracked queued receipts survive annotation"
    );
    let mut conversation = ConversationState::default();
    conversation.replace_history(&history);
    assert_eq!(
        conversation.items.len(),
        1,
        "only the older answer reached the model"
    );
    let pending = conversation.pending_receipts();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].id, "queued-steer");
    assert_eq!(pending[0].mode, Some(PromptMode::Steer));
    assert_eq!(pending[1].id, "queued-follow");
    assert_eq!(pending[1].mode, Some(PromptMode::FollowUp));
    assert!(
        pending
            .iter()
            .all(|receipt| receipt.images.len() == 1 && !receipt.unknown)
    );
    assert!(
        conversation.queue.steering.is_empty() && conversation.queue.follow_up.is_empty(),
        "saved receipts are presentation, not executable input"
    );
    assert!(
        store.queued_prompts()?.is_empty(),
        "native acceptance forbids automatic replay"
    );
    for receipt in &history[1..] {
        assert_eq!(receipt["content"][1]["data"], ONE_PIXEL_PNG);
        conversation.record_prompt_delivery(
            receipt["submissionId"].as_str().unwrap(),
            receipt,
            "delivered",
        );
    }
    assert_eq!(conversation.items.len(), 3);
    assert!(conversation.pending_receipts().is_empty());
    assert!(
        conversation
            .items
            .iter()
            .skip(1)
            .all(|item| item.text == "same text" && item.images.len() == 1)
    );
    Ok(())
}

#[test]
fn reopened_accepted_queue_receipts_stay_off_transcript_until_delivery()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::protocol::{PromptImage, PromptMode};
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("native-session");
    let image = PromptImage::new(ONE_PIXEL_PNG.into(), "image/png".into());
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
        assert_eq!(receipt["content"][1]["data"], ONE_PIXEL_PNG);
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
