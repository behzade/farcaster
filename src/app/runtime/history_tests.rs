use super::*;

#[test]
fn accepted_image_only_prompt_survives_empty_backend_history_and_reopen()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session");
    let image = crate::protocol::PromptImage::new("aGVsbG8=".into(), "image/png".into());
    let mut store = StateStore::open_at(&database)?;
    let id = store.enqueue_prompt(
        "draft:image",
        "codex-cli",
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
    assert_eq!(messages[0]["content"][1], serde_json::to_value(image)?);
    let mut backend = vec![serde_json::json!({"role":"assistant", "content":"existing"})];
    let original = backend.clone();
    annotate_history_presentations(Some(&store), &session, &mut backend);
    assert_eq!(backend, original);
    assert!(
        store
            .accepted_prompt_history(&temp.path().join("other"))?
            .is_empty()
    );
    Ok(())
}
