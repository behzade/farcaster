use super::super::tests::writable_test_session;
use super::*;
use std::io::BufRead;

fn read(sent: &mut BufReader<std::process::ChildStdout>) -> Value {
    let mut line = String::new();
    sent.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn submit(session: &mut CodexWorkerSession, id: &str, command: &str) -> Result<bool, String> {
    session.submit_prompt(
        id.into(),
        command.into(),
        WorkerSendMode::Prompt,
        Vec::new(),
    )
}

fn reply(session: &mut CodexWorkerSession, request: &Value, result: Value) -> Vec<WorkerEvent> {
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: serde_json::from_value(request["id"].clone()).unwrap(),
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
        assert!(!submit(&mut session, "submission", command).unwrap());
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
    submit(&mut session, "choose", "/model chosen high").unwrap();
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
        .unwrap();
    let turn = read(&mut sent);
    assert_eq!(turn["method"], "turn/start");
    assert_eq!(turn["params"]["model"], "chosen");
    assert_eq!(turn["params"]["effort"], "high");
}

#[test]
fn permission_selection_respects_allowed_profiles_and_server_rejections() {
    for allowed in [true, false] {
        let (mut session, mut sent) = writable_test_session();
        submit(&mut session, "choose", "/permissions restricted").unwrap();
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
                id: serde_json::from_value(update["id"].clone()).unwrap(),
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
    submit(&mut session, "status", "/status").unwrap();
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
