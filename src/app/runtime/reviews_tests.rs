use super::*;

fn review_arguments() -> Value {
    json!({
        "title": "Review README",
        "items": [
            {"path": "README.md", "start_line": 1, "end_line": 8, "note": "Title and icon"},
            {"path": "README.md", "start_line": 35, "end_line": 46, "note": "Docs range"}
        ]
    })
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

fn replayed_review_history(arguments: &Value) -> ConversationState {
    let mut state = ConversationState::default();
    state.replace_history(&[
        json!({"role":"user","content":[{"type":"text","text":"use submit review on readme"}]}),
        json!({
            "role":"assistant",
            "content":[{"type":"toolCall","id":"replay-0-5","name":"farcaster: submit_review","arguments":arguments}]
        }),
        json!({"role":"toolResult","toolCallId":"replay-0-5","isError":false,
            "content":[{"type":"text","text":"The review was submitted successfully."}]}),
        json!({"role":"assistant","content":[{"type":"text","text":"Submitted the review card."}]}),
    ]);
    state
}

fn apply_snapshot(snapshot: &mut RuntimeSnapshot) {
    ReviewProjection::default().apply(snapshot);
}

#[test]
fn replayed_submit_rows_render_the_review_from_their_own_arguments() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let mut snapshot = snapshot(temp.path(), replayed_review_history(&review_arguments()));
    snapshot.transcript_changed_from = Some(0);
    apply_snapshot(&mut snapshot);
    assert_eq!(cards(&snapshot), 1, "the replayed row renders the card");
    let document = snapshot.transcript_presentation();
    let item = document
        .items
        .iter()
        .find(|item| artifact::from_item(item).is_some())
        .expect("hydrated review row");
    let artifact = artifact::from_item(item).expect("hydrated artifact");
    assert_eq!(artifact.review.title, "Review README");
    assert_eq!(artifact.review.items.len(), 2);
    assert_eq!(artifact.project, temp.path());
    Ok(())
}

#[test]
fn nested_farcaster_proxy_arguments_render_the_review() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let nested = json!({
        "providerIdentifier": "farcaster",
        "toolName": "submit_review",
        "args": review_arguments()
    });
    let mut snapshot = snapshot(temp.path(), replayed_review_history(&nested));
    snapshot.transcript_changed_from = Some(0);
    apply_snapshot(&mut snapshot);
    assert_eq!(cards(&snapshot), 1);
    Ok(())
}

#[test]
fn live_rows_with_their_own_artifact_render_without_hydration() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let mut state = ConversationState::default();
    state.reduce(&json!({
        "type":"tool_execution_start","toolCallId":"live","toolName":"submit_review",
        "args":{"title":"Review README","items":[{"path":"README.md","note":"Inspect"}]}
    }));
    state.reduce(&json!({
        "type":"tool_execution_end","toolCallId":"live","isError":false,
        "result":{"farcaster_review":{"id":"live","version":1,"project":temp.path(),
            "review":{"title":"Review README","items":[{"path":"README.md","note":"Inspect"}]}}}
    }));
    let mut snapshot = snapshot(temp.path(), state);
    apply_snapshot(&mut snapshot);
    assert_eq!(cards(&snapshot), 1);
    // The live row already carries its artifact, so the presentation keeps
    // sharing the conversation's item unchanged.
    assert!(Arc::ptr_eq(
        snapshot.conversation.items.get(0).expect("tool row"),
        snapshot
            .transcript_presentation()
            .items
            .get(0)
            .expect("presentation tool row")
    ));
    Ok(())
}

#[test]
fn failed_review_rows_render_as_plain_tool_rows() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let mut state = ConversationState::default();
    state.reduce(&json!({
        "type":"tool_execution_start","toolCallId":"failed","toolName":"submit_review",
        "args":{"title":"Review README","items":[{"path":"README.md","note":"Inspect"}]}
    }));
    state.reduce(&json!({
        "type":"tool_execution_end","toolCallId":"failed","result":{"error":"no caller"},"isError":true
    }));
    let mut snapshot = snapshot(temp.path(), state);
    apply_snapshot(&mut snapshot);
    assert_eq!(cards(&snapshot), 0);
    Ok(())
}

#[test]
fn streaming_rebuilds_only_the_changed_suffix_and_keeps_runs() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let mut state = ConversationState::default();
    let prompt = state.push_local_user("Review".into(), 0, false);
    state.bind_submitted_prompt("turn", &prompt);
    state.begin_run();
    message(&mut state, "Streaming");
    let mut snapshot = snapshot(temp.path(), state);
    let mut projection = ReviewProjection::default();
    projection.apply(&mut snapshot);
    assert_eq!(snapshot.transcript_presentation().active_start, Some(1));
    let previous = snapshot.transcript_presentation();
    let last = snapshot.conversation.items.len() - 1;
    let mut tail = snapshot.conversation.items[last].as_ref().clone();
    tail.text.push_str(" delta");
    Arc::make_mut(&mut snapshot.conversation)
        .items
        .set(last, Arc::new(tail));
    snapshot.transcript_changed_from = Some(last);
    projection.apply(&mut snapshot);
    assert!(snapshot.transcript_changed_from.expect("dirty suffix") >= last);
    assert!(Arc::ptr_eq(
        &previous.items[0],
        &snapshot.transcript_presentation().items[0]
    ));
    assert_eq!(cards(&snapshot), 0);
    Ok(())
}

#[test]
fn a_replay_that_later_regains_its_result_rehydrates_away() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let arguments = review_arguments();
    let mut state = replayed_review_history(&arguments);
    let mut snapshot = snapshot(temp.path(), state);
    snapshot.transcript_changed_from = Some(0);
    apply_snapshot(&mut snapshot);
    assert_eq!(cards(&snapshot), 1);
    // The harness may attach the echoing result later; the row then carries
    // its own artifact and the presentation resyncs without duplicating.
    let tool_index = snapshot
        .conversation
        .items
        .position(|item| item.tool_details.is_some())
        .expect("tool row");
    let mut row = snapshot.conversation.items[tool_index].as_ref().clone();
    let details = Arc::make_mut(row.tool_details.as_mut().expect("details"));
    details.result = Some(
        json!({"content":[{"type":"text","text":serde_json::to_string(&json!({
        "farcaster_review":{"id":"late","version":1,"project":temp.path(),"review":{"title":"Review README","items":[{"path":"README.md","note":"Inspect"}]}}
    })).unwrap()}]}),
    );
    details.state = crate::conversation::ToolExecutionState::Succeeded;
    Arc::make_mut(&mut snapshot.conversation)
        .items
        .set(tool_index, Arc::new(row));
    snapshot.transcript_changed_from = Some(tool_index);
    apply_snapshot(&mut snapshot);
    assert_eq!(cards(&snapshot), 1, "no duplicate review rows");
    Ok(())
}

fn message(state: &mut ConversationState, text: &str) {
    let message = json!({"role":"assistant","content":[{"type":"text","text":text}]});
    state.reduce(&json!({"type":"message_start","message":message}));
    state.reduce(&json!({"type":"message_end","message":message}));
}
