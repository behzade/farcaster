use super::super::tests::writable_test_session;
use super::*;
use std::io::BufRead;

fn read(sent: &mut BufReader<std::process::ChildStdout>) -> Value {
    let mut line = String::new();
    sent.read_line(&mut line).expect("read fixture request");
    serde_json::from_str(&line).expect("decode fixture request")
}

fn submit(
    session: &mut CodexWorkerSession,
    id: &str,
    command: &str,
) -> Result<bool, crate::PromptRejection> {
    session.submit_prompt(
        id.into(),
        command.into(),
        WorkerSendMode::Prompt,
        Vec::new(),
    )
}

fn reply(session: &mut CodexWorkerSession, request: &Value, result: Value) -> Vec<WorkerEvent> {
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: serde_json::from_value(request["id"].clone()).expect("request ID"),
        result,
    }));
    std::iter::from_fn(|| session.poll()).collect()
}

#[test]
fn review_and_compact_use_native_requests_and_reject_invalid_arguments() {
    for (command, method, target) in [
        ("/compact", "thread/compact/start", Value::Null),
        (
            "/review",
            "review/start",
            json!({"type":"uncommittedChanges"}),
        ),
        (
            "/review branch main",
            "review/start",
            json!({"type":"baseBranch","branch":"main"}),
        ),
        (
            "/review commit abc",
            "review/start",
            json!({"type":"commit","sha":"abc"}),
        ),
    ] {
        let (mut session, mut sent) = writable_test_session();
        assert!(!submit(&mut session, "submission", command).expect("submit command"));
        let request = read(&mut sent);
        assert_eq!(request["method"], method);
        assert_eq!(request["params"]["target"], target);
        assert!(session.poll_prompt_ack().is_none());
        reply(
            &mut session,
            &request,
            if method == "review/start" {
                json!({"reviewThreadId":"thread-1","turn":{"id":"review-turn","status":"inProgress"}})
            } else {
                json!({})
            },
        );
        assert_eq!(
            session.poll_prompt_ack(),
            Some(("submission".into(), Ok(())))
        );
    }
    let (mut session, _) = writable_test_session();
    for command in [
        "/compact extra",
        "/review branch",
        "/model a b c",
        "/permissions a b",
        "/status extra",
    ] {
        assert!(submit(&mut session, "bad", command).is_err());
    }
    assert_eq!(session.next_id, 0);
}

#[test]
fn model_selection_waits_for_update_and_applies_to_subsequent_turns() {
    let (mut session, mut sent) = writable_test_session();
    submit(&mut session, "choose", "/model chosen high").expect("select model");
    let list = read(&mut sent);
    assert_eq!(list["method"], "model/list");
    reply(
        &mut session,
        &list,
        json!({"data":[],"nextCursor":"page-2"}),
    );
    let page = read(&mut sent);
    assert_eq!(page["params"]["cursor"], "page-2");
    reply(
        &mut session,
        &page,
        json!({"data":[{"id":"chosen","displayName":"Chosen","supportedReasoningEfforts":[{"reasoningEffort":"high"}]}]}),
    );
    let update = read(&mut sent);
    assert_eq!(update["method"], "thread/settings/update");
    assert_eq!(
        update["params"],
        json!({"threadId":"thread-1","model":"chosen","effort":"high"})
    );
    assert!(session.poll_prompt_ack().is_none());
    assert!(session.model.is_none());
    let events = reply(&mut session, &update, json!({}));
    assert!(events.iter().any(|event| matches!(event, WorkerEvent::Activity(WorkerActivity::ConfigurationChanged { selected_model:Some(model), .. }) if model["id"] == "chosen")));
    assert_eq!(session.poll_prompt_ack(), Some(("choose".into(), Ok(()))));
    session
        .send("hello".into(), WorkerSendMode::Prompt)
        .expect("send prompt");
    let turn = read(&mut sent);
    assert_eq!(turn["method"], "turn/start");
    assert_eq!(turn["params"]["model"], "chosen");
    assert_eq!(turn["params"]["effort"], "high");
}

#[test]
fn permission_selection_respects_allowed_profiles_and_server_rejections() {
    for allowed in [true, false] {
        let (mut session, mut sent) = writable_test_session();
        submit(&mut session, "choose", "/permissions restricted").expect("select permissions");
        let list = read(&mut sent);
        assert_eq!(list["method"], "permissionProfile/list");
        assert_eq!(list["params"]["cwd"], "/project");
        reply(
            &mut session,
            &list,
            json!({"data":[{"id":"restricted","allowed":allowed}]}),
        );
        if allowed {
            let update = read(&mut sent);
            assert_eq!(
                update["params"],
                json!({"threadId":"thread-1","permissions":"restricted"})
            );
            assert!(session.poll_prompt_ack().is_none());
            session.queued_inbound.push_back(Ok(CodexInbound::Error {
                id: serde_json::from_value(update["id"].clone()).expect("update ID"),
                error: super::super::super::contract::CodexRpcError {
                    code: -1,
                    message: "denied".into(),
                    data: Value::Null,
                },
            }));
            session.poll();
        }
        assert!(matches!(session.poll_prompt_ack(), Some((_, Err(_)))));
        assert!(session.command_state.permissions.is_none());
    }
}

#[test]
fn status_reports_usage_and_review_completion_retains_native_output() {
    let (mut session, mut sent) = writable_test_session();
    session.observe_command_settings(&json!({"model":"chosen","modelProvider":"openai","effort":"high","activePermissionProfile":{"id":"restricted"}}));
    session.command_state.usage = Some(WorkerUsage {
        turn: TokenUsage {
            input: 42,
            ..Default::default()
        },
        context_window: 1000,
        ..Default::default()
    });
    submit(&mut session, "status", "/status").expect("request status");
    let request = read(&mut sent);
    assert_eq!(request["method"], "account/rateLimits/read");
    let events = reply(
        &mut session,
        &request,
        json!({"rateLimits":{"primary":{"usedPercent":25}}}),
    );
    assert!(events.iter().any(|event| matches!(event, WorkerEvent::Settled { output } if output.contains("42 / 1000") && output.contains("25% used") && output.contains("chosen") && output.contains("restricted"))));
    session.current_turn = Some("review".into());
    session.queued_inbound.push_back(Ok(CodexInbound::Notification {
        method:"item/completed".into(), params:json!({"threadId":"thread-1","item":{"type":"exitedReviewMode","review":"No issues found."}}),
    }));
    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-1","turn":{"id":"review","status":"completed"}}),
        }));
    assert_eq!(
        session.poll(),
        Some(WorkerEvent::Settled {
            output: "No issues found.".into()
        })
    );
}

#[test]
fn local_commands_settle_submissions_without_a_model_message() {
    use crate::adapter::main_session::{MainSessionMetadata, WorkerSessionTransport};
    use crate::extensions::PromptMode;
    use crate::{DeliveryStatus, SessionCommand, SessionEvent, SessionTransport};

    for command in [
        "/model",
        "/model chosen high",
        "/permissions restricted",
        "/status",
    ] {
        let (mut session, mut sent) = writable_test_session();
        let (sender, incoming) = std::sync::mpsc::channel();
        session.incoming = incoming;
        let mut transport = WorkerSessionTransport::new(
            std::path::Path::new("/locators"),
            Backend::Codex,
            "thread-1".into(),
            Box::new(session),
            MainSessionMetadata::default(),
            None,
        )
        .expect("transport");
        let id = transport
            .send(SessionCommand::Prompt {
                mode: PromptMode::Normal,
                message: command.into(),
                images: Vec::new(),
            })
            .expect("submit local command");
        let request = read(&mut sent);
        let result = match command {
            "/permissions restricted" => json!({"data":[{"id":"restricted","allowed":true}]}),
            "/status" => json!({"rateLimits":{}}),
            _ => {
                json!({"data":[{"id":"chosen","displayName":"Chosen","supportedReasoningEfforts":[{"reasoningEffort":"high"}]}]})
            }
        };
        sender
            .send(Ok(CodexInbound::Response {
                id: serde_json::from_value(request["id"].clone()).unwrap(),
                result,
            }))
            .unwrap();
        let mut events: Vec<_> = std::iter::from_fn(|| transport.poll()).collect();
        if matches!(command, "/model chosen high" | "/permissions restricted") {
            assert!(events.iter().all(|event| !matches!(event, SessionEvent::Activity(event) if event.value()["type"] == "prompt_delivery")), "{command} must wait for settings update");
            let update = read(&mut sent);
            assert_eq!(update["method"], "thread/settings/update");
            sender
                .send(Ok(CodexInbound::Response {
                    id: serde_json::from_value(update["id"].clone()).unwrap(),
                    result: json!({}),
                }))
                .unwrap();
            events.extend(std::iter::from_fn(|| transport.poll()));
        }
        let deliveries: Vec<_> = events
            .iter()
            .filter_map(|event| {
                let SessionEvent::Activity(event) = event else {
                    return None;
                };
                serde_json::from_value::<crate::contract::PromptDelivery>(event.value().clone())
                    .ok()
            })
            .filter(|receipt| {
                receipt.submission_id == id && receipt.status == DeliveryStatus::Delivered
            })
            .collect();
        assert_eq!(
            deliveries.len(),
            1,
            "{command} must settle the exact submission"
        );
        assert!(
            deliveries[0].message.is_none(),
            "local completion must not invent a model user-message echo"
        );
        assert_eq!(events.iter().filter(|event| matches!(event, SessionEvent::Response(response) if response.id.as_deref() == Some(&id) && response.result.is_ok())).count(), 1);

        let prompt = transport
            .send(SessionCommand::Prompt {
                mode: PromptMode::Normal,
                message: "real prompt".into(),
                images: Vec::new(),
            })
            .expect("submit model prompt after command");
        let turn = read(&mut sent);
        assert_eq!(turn["method"], "turn/start");
        sender
            .send(Ok(CodexInbound::Response {
                id: serde_json::from_value(turn["id"].clone()).unwrap(),
                result: json!({"turn":{"id":"real-turn","status":"inProgress"}}),
            }))
            .unwrap();
        let events: Vec<_> = std::iter::from_fn(|| transport.poll()).collect();
        assert!(
            events.iter().all(|event| {
                let SessionEvent::Activity(event) = event else {
                    return true;
                };
                let Ok(receipt) = serde_json::from_value::<crate::contract::PromptDelivery>(
                    event.value().clone(),
                ) else {
                    return true;
                };
                receipt.submission_id != prompt || receipt.status != DeliveryStatus::Delivered
            }),
            "ordinary model admission must still wait for native delivery"
        );
    }
}
