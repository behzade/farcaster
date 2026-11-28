use super::*;
use crate::runtime::tests::owner_without_process;

fn emit(owner: &mut RuntimeOwner, json: Value) {
    owner.apply_process_item(SessionEvent::Activity(json.into()));
}

#[test]
fn live_transcript_binds_before_checkpoint_and_reopens_streams_without_duplicate_tools() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().canonicalize().unwrap();
    let path = project.join("session.jsonl");
    let db = project.join("state.sqlite3");
    let store = SharedStateStore::open_at(&db).unwrap();
    let (mut owner, _) = owner_without_process(project.clone());
    owner.state = Some(store.clone());
    owner.live_transcript = Some(LiveTranscript::new(
        Backend::Pi,
        None,
        &project,
        None,
        true,
        &store,
    ));
    emit(
        &mut owner,
        json!({"type":"prompt_delivery","submissionId":"p1","status":"accepted","message":{"role":"user","content":"question"}}),
    );
    emit(
        &mut owner,
        json!({"type":"prompt_delivery","submissionId":"p1","status":"delivered","message":null}),
    );
    emit(
        &mut owner,
        json!({"type":"message_start","message":{"role":"assistant","content":[]}}),
    );
    emit(
        &mut owner,
        json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"par"}}),
    );
    owner.flush_live_transcript();
    owner.apply_response(agents::SessionResponse::success(
        None,
        agents::SessionResponsePayload::LoadState(
            serde_json::from_value(json!({
                "sessionFile":path,"sessionId":"native","isStreaming":true,"isCompacting":false,
                "autoCompactionEnabled":true,"messageCount":1,"pendingMessageCount":0
            }))
            .unwrap(),
        ),
    ));
    let session = TranscriptSession {
        harness: Backend::Pi,
        path: &path,
        project: &project,
        profile_id: None,
    };
    owner.flush_live_transcript();
    let partial = store.load_transcript(session, None).unwrap().unwrap();
    assert_eq!(partial.messages.len(), 2);
    assert_eq!(partial.messages[0]["submissionId"], "p1");
    assert_eq!(partial.messages[1]["content"][0]["text"], "par");
    emit(
        &mut owner,
        json!({"type":"message_end","message":{"role":"assistant","nativeMessageId":"native-2","content":[{"type":"text","text":"part two"},{"type":"toolCall","id":"c1","name":"shell","arguments":{"command":"pwd"}}]}}),
    );
    emit(
        &mut owner,
        json!({"type":"tool_execution_start","toolCallId":"c1","toolName":"shell","args":{"command":"pwd"}}),
    );
    emit(
        &mut owner,
        json!({"type":"tool_execution_update","toolCallId":"c1","partialResult":{"content":[{"type":"text","text":"building"}]}}),
    );
    owner.flush_live_transcript();
    let running = store.load_transcript(session, None).unwrap().unwrap();
    assert_eq!(running.messages.len(), 3);
    assert_eq!(running.messages[2]["farcasterToolState"], "running");
    emit(
        &mut owner,
        json!({"type":"tool_execution_end","toolCallId":"c1","isError":false,"result":{"content":[{"type":"text","text":"finished"}],"details":{"exitCode":0}}}),
    );
    owner.reset_process_runtime();
    drop(owner);
    drop(store);

    let store = SharedStateStore::open_at(&db).unwrap();
    let document = store.load_transcript(session, None).unwrap().unwrap();
    assert!(document.complete);
    assert_eq!(document.messages.len(), 3);
    assert_eq!(document.messages[1]["nativeMessageId"], "native-2");
    assert_eq!(document.messages[2]["details"]["exitCode"], 0);
    let mut convo = ConversationState::default();
    convo.replace_history(&document.messages);
    assert_eq!(convo.items.len(), 3);
    assert_eq!(convo.items[1].complete_text(), "part two");
    assert_eq!(convo.items[2].tool_output, "finished");
    let details = convo.items[2].tool_details.as_ref().unwrap();
    assert_eq!(details.arguments["command"], "pwd");
    assert_eq!(
        details.state,
        crate::conversation::ToolExecutionState::Succeeded
    );

    convo.replace_history(&document.messages[2..]);
    assert_eq!(convo.items.len(), 1);
    assert_eq!(convo.items[0].tool_output, "finished");
    assert_eq!(
        convo.items[0].tool_details.as_ref().unwrap().arguments["command"],
        "pwd"
    );
}

#[test]
fn live_transcript_resume_preserves_newer_database_import_and_keeps_the_actor_address() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path();
    let path = project.join("session");
    let other = project.join("also-open");
    let store = SharedStateStore::open_at(&project.join("state.sqlite")).unwrap();
    let session = TranscriptSession {
        harness: Backend::Pi,
        path: &path,
        project,
        profile_id: None,
    };
    let mut capture = LiveTranscript::new(Backend::Pi, Some(&path), project, None, false, &store);
    let prefix =
        json!({"role":"assistant", "content":[{"type":"text","text":"new imported prefix"}]});
    assert!(
        store
            .replace_transcript(session, 0, std::slice::from_ref(&prefix), &json!({}))
            .unwrap()
    );
    capture.replace_startup_history(&Arc::new(vec![
        json!({"role":"user","content":"stale startup read"}),
    ]));
    let (mut owner, _) = owner_without_process(project.into());
    owner.state = Some(store.clone());
    owner.live_transcript = Some(capture);
    owner.snapshot.selected_session = Some(other.clone());
    emit(
        &mut owner,
        json!({"type":"message_end","message":{"role":"assistant","nativeMessageId":"m2","content":[{"type":"text","text":"live response"}]}}),
    );
    owner.flush_live_transcript();
    let document = store.load_transcript(session, None).unwrap().unwrap();
    assert_eq!(document.messages.len(), 2);
    assert_eq!(document.messages[0], prefix);
    assert_eq!(document.messages[1]["nativeMessageId"], "m2");
    assert!(
        store
            .load_transcript(
                TranscriptSession {
                    path: &other,
                    ..session
                },
                None
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn live_transcript_reopen_omits_unconfirmed_sends_and_honors_an_empty_native_history() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path();
    let db = project.join("state.sqlite");
    let path = project.join("session");
    let session = TranscriptSession {
        harness: Backend::Pi,
        path: &path,
        project,
        profile_id: None,
    };
    let store = SharedStateStore::open_at(&db).unwrap();
    assert!(
        store
            .replace_transcript(
                session,
                0,
                &[json!({"role":"user","content":"removed by undo"})],
                &json!({})
            )
            .unwrap()
    );
    let mut capture = LiveTranscript::new(Backend::Pi, Some(&path), project, None, false, &store);
    capture.replace_startup_history(&Arc::default());
    for (id, status, queued) in [
        ("a", "rejected", false),
        ("b", "cancelled", false),
        ("c", "unknown", false),
        ("q", "accepted", true),
        ("q", "cancelled", true),
        ("z", "accepted", true),
    ] {
        capture.observe(&json!({"type":"prompt_delivery","submissionId":id,"status":status,"message":{"role":"user","content":"same prompt","queued":queued}}).into());
    }
    capture.flush(&store).unwrap();
    assert!(
        store
            .load_transcript(session, None)
            .unwrap()
            .unwrap()
            .messages
            .is_empty()
    );

    // A queued receipt becomes visible only after delivery. An uncorrelated old
    // message with the same text stays separate; a matching native ID adds provenance.
    capture.observe(&json!({"type":"prompt_delivery","submissionId":"z","status":"delivered","message":{"role":"user","content":"same prompt","queued":true,"nativeMessageId":"one"}}).into());
    capture.observe(&json!({"type":"message_end","message":{"role":"user","content":"same prompt","nativeMessageId":"one"}}).into());
    capture.observe(
        &json!({"type":"message_end","message":{"role":"user","content":"same prompt"}}).into(),
    );
    capture.flush(&store).unwrap();
    drop(store);
    let reopened = SharedStateStore::open_at(&db).unwrap();
    let messages = reopened
        .load_transcript(session, None)
        .unwrap()
        .unwrap()
        .messages;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["submissionId"], "z");
    assert_eq!(messages[0]["nativeMessageId"], "one");
    assert!(messages[1].get("submissionId").is_none());
}
