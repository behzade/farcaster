use super::*;

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
        "pi",
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
    let mut conversation =
        crate::app::views::transcript::conversation::ConversationState::default();
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
        "codex-cli",
        temp.path(),
        None,
        crate::protocol::PromptMode::Normal,
        "first",
        &[],
    )?;
    let second = store.enqueue_prompt(
        "draft:second",
        "codex-cli",
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
        "codex-cli",
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
    let mut conversation =
        crate::app::views::transcript::conversation::ConversationState::default();
    conversation.replace_history(&history);
    assert_eq!(conversation.items.len(), 2);
    assert_eq!(conversation.items[1].text, "second");
    assert_eq!(conversation.items[1].images.len(), 1);
    Ok(())
}
