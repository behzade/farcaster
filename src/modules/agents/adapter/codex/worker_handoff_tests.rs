use super::*;

fn request(reader: &mut impl std::io::BufRead) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).expect("read fixture request");
    serde_json::from_str(&line).expect("decode fixture request")
}

fn reply(session: &mut CodexWorkerSession, request: &Value, result: Value) {
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: serde_json::from_value(request["id"].clone()).expect("request ID"),
        result,
    }));
    while session.poll().is_some() {}
}

fn claimed_batch(
    active_turn: bool,
) -> (
    CodexWorkerSession,
    std::io::BufReader<std::process::ChildStdout>,
    Value,
) {
    let (mut session, mut sent) = writable_test_session();
    session.native_queue = true;
    session.current_turn = Some("turn-1".into());
    for (index, image) in ["aGVsbG8=", "d29ybGQ="].into_iter().enumerate() {
        let submission_id = format!("accepted-{index}");
        session
            .submit_prompt(
                submission_id.clone(),
                "inspect image".into(),
                WorkerSendMode::Queue,
                vec![crate::protocol::PromptImage::new(
                    image.into(),
                    "image/png".into(),
                )],
            )
            .expect("submit queued prompt");
        let queued = request(&mut sent);
        reply(
            &mut session,
            &queued,
            json!({"queuedSubmission": {
                "id":format!("queue-{index}"),
                "clientUserMessageId":queued["params"]["clientUserMessageId"], "input":[]
            }}),
        );
        assert_eq!(session.poll_prompt_ack(), Some((submission_id, Ok(()))));
    }
    session.apply_steering().expect("apply steering");
    let interrupt = request(&mut sent);
    reply(&mut session, &interrupt, json!({}));
    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-1","turn":{"id":"turn-1","status":"interrupted"}}),
        }));
    while session.poll().is_some() {}
    let deletions = [request(&mut sent), request(&mut sent)];
    if active_turn {
        session.current_turn = Some("auto-started-turn".into());
    }
    for deletion in deletions {
        assert_eq!(deletion["method"], "thread/queue/delete");
        reply(&mut session, &deletion, json!({"deleted":true}));
    }
    let batch = request(&mut sent);
    (session, sent, batch)
}

fn reject(session: &mut CodexWorkerSession, batch: &Value) {
    session.queued_inbound.push_back(Ok(CodexInbound::Error {
        id: serde_json::from_value(batch["id"].clone()).expect("batch ID"),
        error: super::super::super::contract::CodexRpcError {
            code: -32000,
            message: "handoff rejected".into(),
            data: Value::Null,
        },
    }));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::RequestFailed { .. })
    ));
}

#[test]
fn rejected_claimed_handoff_retries_exact_input_only_on_explicit_apply() {
    for active_turn in [false, true] {
        let (mut session, mut sent, batch) = claimed_batch(active_turn);
        assert_eq!(
            batch["method"],
            if active_turn {
                "turn/steer"
            } else {
                "turn/start"
            }
        );
        reject(&mut session, &batch);
        let before = session.next_id;
        while session.poll().is_some() {}
        assert_eq!(session.next_id, before, "rejection must not auto-retry");
        assert_eq!(
            session.poll_prompt_ack(),
            None,
            "prior admission must not be undone"
        );
        // A now-idle turn lets the explicit retry start immediately.
        session.current_turn = None;
        session.apply_steering().expect("apply steering");
        assert!(
            session.next_id > before,
            "accepted input was dropped instead of retained"
        );
        let retry = request(&mut sent);
        assert_eq!(retry["method"], "turn/start");
        assert_eq!(retry["params"]["input"], batch["params"]["input"]);
        reply(
            &mut session,
            &retry,
            json!({"turn":{"id":"retry-turn","status":"inProgress"}}),
        );
        assert_eq!(session.poll_prompt_ack(), None);
        for method in ["item/started", "item/completed"] {
            session
                .queued_inbound
                .push_back(Ok(CodexInbound::Notification {
                    method: method.into(),
                    params: json!({"threadId":"thread-1","turnId":"retry-turn","item": {
                        "type":"userMessage", "clientId":retry["params"]["clientUserMessageId"],
                        "content":retry["params"]["input"]
                    }}),
                }));
        }
        let mut delivered = Vec::new();
        while let Some(event) = session.poll() {
            if let WorkerEvent::Activity(WorkerActivity::SubmittedInputDeliveredWithImages {
                submission_id,
                images,
                ..
            }) = event
            {
                delivered.push((submission_id, images[0].data.clone()));
            }
        }
        assert_eq!(
            delivered,
            [
                ("accepted-0".into(), "aGVsbG8=".into()),
                ("accepted-1".into(), "d29ybGQ=".into()),
            ],
            "equal-text inputs must retain order and distinct delivery identities"
        );
    }
}

#[test]
fn abort_discards_rejected_handoff_instead_of_retrying_it() {
    for abort_before_rejection in [false, true] {
        let (mut session, mut sent, batch) = claimed_batch(false);
        if abort_before_rejection {
            session.abort().expect("abort session");
        }
        reject(&mut session, &batch);
        if !abort_before_rejection {
            session.abort().expect("abort session");
            let cleanup = request(&mut sent);
            assert_eq!(cleanup["method"], "thread/backgroundTerminals/clean");
            reply(&mut session, &cleanup, json!({}));
        }
        let before = session.next_id;
        session.apply_steering().expect("apply steering");
        assert_eq!(session.next_id, before);
    }
}

#[test]
fn rejected_unacknowledged_handoff_returns_input_to_caller_without_local_retry() {
    let (mut session, mut sent) = writable_test_session();
    session.current_turn = Some("turn-1".into());
    session
        .submit_prompt(
            "pending-steer".into(),
            "not yet admitted".into(),
            WorkerSendMode::Steer,
            Vec::new(),
        )
        .expect("submit steering prompt");
    let steer = request(&mut sent);
    session.apply_steering().expect("apply steering");
    let interrupt = request(&mut sent);
    reply(&mut session, &interrupt, json!({}));
    session.queued_inbound.push_back(Ok(CodexInbound::Error {
        id: serde_json::from_value(steer["id"].clone()).expect("steering request ID"),
        error: super::super::super::contract::CodexRpcError {
            code: -32000,
            message: "no active turn to steer".into(),
            data: Value::Null,
        },
    }));
    while session.poll().is_some() {}
    assert_eq!(session.poll_prompt_ack(), None);
    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-1","turn":{"id":"turn-1","status":"interrupted"}}),
        }));
    while session.poll().is_some() {}
    let batch = request(&mut sent);
    reject(&mut session, &batch);
    assert!(matches!(session.poll_prompt_ack(), Some((id, Err(_))) if id == "pending-steer"));
    let before = session.next_id;
    session.apply_steering().expect("apply steering");
    assert_eq!(
        session.next_id, before,
        "caller already received the rejected input"
    );
}
