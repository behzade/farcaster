use super::*;

fn caller(project: &std::path::Path) -> crate::agents::CallerContext {
    crate::agents::CallerContext {
        worker_id: "worker".into(),
        worker_name: "Worker".into(),
        project: project.into(),
        session: "native".into(),
        backend: Backend::Cursor,
        provider: None,
        model: None,
        effort: None,
        access_mode: crate::agents::HarnessAccessMode::Auto,
        parent_worker_id: None,
    }
}
fn save(store: &StateStore, project: &std::path::Path, id: &str, ordinal: Option<usize>) -> Value {
    let caller = caller(project);
    let execution = crate::agents::ExecutionBinding {
        session_record: store.register_caller_session(&caller).expect("session"),
        turn_id: id.into(),
        prompt_id: Some(id.into()),
    };
    store.register_execution(&execution).expect("execution");
    if let Some(ordinal) = ordinal {
        store.record_review_position(id, ordinal).expect("position");
    }
    let artifact = json!({"farcaster_review":{"id":id,"version":1,"project":project,"review":{"title":"Review README","items":[{"path":"README.md","note":"Inspect"}]}}});
    store
        .save_review(&caller, &execution, &artifact)
        .expect("review");
    artifact
}
fn snapshot(project: &std::path::Path, state: ConversationState) -> RuntimeSnapshot {
    RuntimeSnapshot {
        harness: Some(Backend::Cursor),
        project: project.into(),
        selected_session: Some(project.join("session-locators/cursor-cli/native")),
        conversation: Arc::new(state),
        ..Default::default()
    }
}
fn cards(snapshot: &RuntimeSnapshot) -> usize {
    snapshot
        .transcript_presentation()
        .items
        .iter()
        .filter(|item| artifact::from_item(item).is_some())
        .count()
}
fn tool(state: &mut ConversationState, id: &str, output: Value) {
    state.reduce(&json!({"type":"tool_execution_start","toolCallId":id,"toolName":"submit_review","args":{}}));
    state.reduce(
        &json!({"type":"tool_execution_end","toolCallId":id,"result":output,"isError":false}),
    );
}

#[test]
fn history_load_rebinds_saved_review_cards_onto_the_reloaded_transcript() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    save(&store, temp.path(), "turn", None);
    let mut snapshot = snapshot(temp.path(), ConversationState::default());
    let mut projection = ReviewProjection::default();
    projection.apply(Some(&store), &mut snapshot);
    assert_eq!(cards(&snapshot), 1, "empty preview shows the saved card");
    let mut loaded = ConversationState::default();
    loaded.replace_history(&[
        json!({"role":"user","content":[{"type":"text","text":"use submit review on readme"}]}),
        json!({"role":"assistant","content":[{"type":"text","text":"Submitted review"}]}),
    ]);
    snapshot.conversation = Arc::new(loaded);
    snapshot.transcript_changed_from = Some(0);
    projection.apply(Some(&store), &mut snapshot);
    assert_eq!(
        cards(&snapshot),
        1,
        "loaded history keeps the review button row"
    );
    Ok(())
}

#[test]
fn history_load_keeps_the_review_row_when_the_native_submit_row_replays() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let artifact = save(&store, temp.path(), "turn", Some(0));
    let mut snapshot = snapshot(temp.path(), ConversationState::default());
    let mut projection = ReviewProjection::default();
    projection.apply(Some(&store), &mut snapshot);
    assert_eq!(cards(&snapshot), 1, "empty preview shows the saved card");
    // The Antigravity replay can include the executed submit_review tool call
    // with its farcaster_review result; the native row then represents the card
    // itself and must render the review row instead of a bare tool row.
    let mut loaded = ConversationState::default();
    loaded.replace_history(&[
        json!({"role":"user","content":[{"type":"text","text":"use submit review on readme"}]}),
        json!({
            "role":"assistant",
            "content":[{"type":"toolCall","id":"native-review","name":"submit_review","arguments":{}}]
        }),
        json!({"role":"toolResult","toolCallId":"native-review","isError":false,
            "content":[{"type":"text","text":artifact.to_string()}]}),
        json!({"role":"assistant","content":[{"type":"text","text":"Submitted review"}]}),
    ]);
    snapshot.conversation = Arc::new(loaded);
    snapshot.transcript_changed_from = Some(0);
    projection.apply(Some(&store), &mut snapshot);
    assert_eq!(
        cards(&snapshot),
        1,
        "the replayed native submit row keeps representing the review"
    );
    Ok(())
}

fn message(state: &mut ConversationState, text: &str) {
    let message = json!({"role":"assistant","content":[{"type":"text","text":text}]});
    state.reduce(&json!({"type":"message_start","message":message}));
    state.reduce(&json!({"type":"message_end","message":message}));
}

#[test]
fn stripped_results_render_once_in_presentation_without_mutating_the_reducer() -> Result<(), String>
{
    for output in [
        json!({"success":true}),
        json!({"content":[{"type":"text","text":"Submit review"}]}),
    ] {
        let temp = tempfile::tempdir().expect("project");
        let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
        let artifact = save(&store, temp.path(), "turn", None);
        let mut source = ConversationState::default();
        let prompt = source.push_local_user("Review".into(), 0, false);
        source.bind_submitted_prompt("turn", &prompt);
        source.begin_run();
        tool(&mut source, "native", output);
        let mut snapshot = snapshot(temp.path(), source);
        let original = snapshot.conversation.clone();
        let mut projection = ReviewProjection::default();
        projection.apply(Some(&store), &mut snapshot);
        assert_eq!(cards(&snapshot), 1);
        assert!(Arc::ptr_eq(&original, &snapshot.conversation));
        assert_eq!(snapshot.transcript_presentation().active_start, Some(1));
        let persisted = store.session_reviews(
            Backend::Cursor,
            temp.path(),
            snapshot.selected_session.as_deref().expect("session"),
        )?;
        assert_eq!(persisted[0].user_ordinal, Some(0));
        tool(
            Arc::make_mut(&mut snapshot.conversation),
            "echo",
            json!({"content":[{"type":"text","text":artifact.to_string()}]}),
        );
        snapshot.transcript_changed_from = Some(2);
        projection.apply(Some(&store), &mut snapshot);
        assert_eq!(cards(&snapshot), 1);
    }
    Ok(())
}

#[test]
fn incremental_projection_preserves_history_and_does_not_refetch_after_arc_replacement()
-> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let database = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    save(&store, temp.path(), "old-turn", Some(0));
    for ordinal in 1..200 {
        save(
            &store,
            temp.path(),
            &format!("old-{ordinal}"),
            Some(ordinal),
        );
    }
    let mut source = ConversationState::default();
    for _ in 0..5000 {
        source.push_local_user("Historical prompt".into(), 0, false);
        message(&mut source, "Historical response");
    }
    source.push_local_user("Current prompt".into(), 0, false);
    source.begin_run();
    message(&mut source, "Streaming");
    let mut snapshot = snapshot(temp.path(), source);
    let mut projection = ReviewProjection::default();
    projection.apply(Some(&store), &mut snapshot);
    let previous = snapshot.transcript_presentation();
    let last = snapshot.conversation.items.len() - 1;
    let mut tail = snapshot.conversation.items[last].as_ref().clone();
    tail.text.push_str(" delta");
    Arc::make_mut(&mut snapshot.conversation)
        .items
        .set(last, Arc::new(tail));
    snapshot.transcript_changed_from = Some(last);
    projection.scanned_items = 0;
    projection.visited_cards = 0;
    projection.apply(Some(&store), &mut snapshot);
    assert_eq!(
        projection.scanned_items, 1,
        "streaming must not scan historical items"
    );
    assert_eq!(
        projection.visited_cards, 0,
        "streaming must reuse historical review placements"
    );
    assert!(Arc::ptr_eq(
        &previous.insertions,
        &snapshot.transcript_presentation().insertions
    ));
    assert!(snapshot.transcript_changed_from.expect("dirty suffix") >= last);
    assert!(Arc::ptr_eq(
        &previous.items[2],
        &snapshot.transcript_presentation().items[2]
    ));
    // A source re-projection is not a database revision. Removing this table
    // makes any accidental lost-Arc refetch fail instead of hiding it.
    rusqlite::Connection::open(&database)
        .expect("connection")
        .execute("DROP TABLE session_reviews", [])
        .expect("remove test table");
    let mut head = snapshot.conversation.items[0].as_ref().clone();
    head.label = "annotated".into();
    Arc::make_mut(&mut snapshot.conversation)
        .items
        .set(0, Arc::new(head));
    snapshot.transcript_changed_from = Some(0);
    projection.apply(Some(&store), &mut snapshot);
    assert_eq!(cards(&snapshot), 200);
    Ok(())
}

#[test]
fn delayed_prompt_binding_relocates_a_card_without_new_journal_data() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    save(&store, temp.path(), "delayed", None);
    let mut source = ConversationState::default();
    let first = source.push_local_user("First".into(), 0, false);
    message(&mut source, "First response");
    source.push_local_user("Second".into(), 0, false);
    message(&mut source, "Second response");
    let mut snapshot = snapshot(temp.path(), source);
    let mut projection = ReviewProjection::default();
    projection.apply(Some(&store), &mut snapshot);
    assert!(artifact::from_item(&snapshot.transcript_presentation().items[4]).is_some());
    Arc::make_mut(&mut snapshot.conversation).bind_submitted_prompt("delayed", &first);
    snapshot.transcript_changed_from = Some(4);
    projection.apply(Some(&store), &mut snapshot);
    assert!(artifact::from_item(&snapshot.transcript_presentation().items[2]).is_some());
    assert_eq!(snapshot.transcript_changed_from, Some(2));
    Ok(())
}

#[test]
fn restored_positions_are_explicit_and_repeated_text_is_not_used_as_identity() -> Result<(), String>
{
    let temp = tempfile::tempdir().expect("project");
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    save(&store, temp.path(), "first", Some(0));
    save(&store, temp.path(), "second", Some(1));
    let mut source = ConversationState::default();
    for _ in 0..3 {
        source.push_local_user("Identical".into(), 0, false);
        message(&mut source, "Done");
    }
    let mut snapshot = snapshot(temp.path(), source);
    let mut projection = ReviewProjection::default();
    projection.apply(Some(&store), &mut snapshot);
    let document = snapshot.transcript_presentation();
    assert_eq!(
        artifact::from_item(&document.items[2])
            .expect("first")
            .id
            .as_deref(),
        Some("first")
    );
    assert_eq!(
        artifact::from_item(&document.items[5])
            .expect("second")
            .id
            .as_deref(),
        Some("second")
    );
    snapshot.selected_session = Some(temp.path().join("session-locators/cursor-cli/other"));
    snapshot.transcript = None;
    projection.apply(Some(&store), &mut snapshot);
    assert_eq!(cards(&snapshot), 0);
    Ok(())
}

#[test]
fn optimistic_ui_edits_keep_review_rows_out_of_protocol_state() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    save(&store, temp.path(), "review", Some(0));
    let mut source = ConversationState::default();
    source.push_local_user("First".into(), 0, false);
    message(&mut source, "Done");
    let mut snapshot = snapshot(temp.path(), source);
    ReviewProjection::default().apply(Some(&store), &mut snapshot);
    let before = snapshot.conversation.items.len();
    Arc::make_mut(&mut snapshot.conversation).push_local_user("Next".into(), 0, false);
    let document = snapshot.transcript.as_mut().expect("presentation");
    Arc::make_mut(document).update_source(&snapshot.conversation, before);
    assert_eq!(cards(&snapshot), 1);
    assert_eq!(snapshot.conversation.items.len(), 3);
    assert_eq!(snapshot.transcript_presentation().items.len(), 4);
    Ok(())
}
