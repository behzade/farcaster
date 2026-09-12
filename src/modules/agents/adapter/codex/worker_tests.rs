use super::*;

#[test]
fn worker_factory_resumes_the_saved_thread_and_accepts_a_new_prompt() -> Result<(), String> {
    const SCRIPT: &str = r#"#!/bin/sh
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$0.requests"
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  [ -n "$id" ] || continue
  case "$line" in
    *'"method":"initialize"'*) result='{"userAgent":"fixture","codexHome":"/tmp/codex-fixture","platformFamily":"unix","platformOs":"macos"}' ;;
    *'"method":"thread/resume"'*) result='{"thread":{"id":"saved-thread","cwd":"/project"},"cwd":"/project"}' ;;
    *'"method":"skills/list"'*) result='{"data":[]}' ;;
    *'"method":"turn/start"'*) result='{"turn":{"id":"new-turn","status":"inProgress"}}' ;;
    *) result='{}' ;;
  esac
  printf '{"id":%s,"result":%s}\n' "$id" "$result"
done
"#;
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let script = project.path().join("codex-resume-fixture.sh");
    std::fs::write(&script, SCRIPT).map_err(|error| error.to_string())?;
    let factory = CodexWorkerFactory::new(AgentLaunchConfig::test_script(&script, Vec::new()));
    let mut worker = factory.create(WorkerLaunch {
        slot: None,
        worker_id: "resumed-worker".into(),
        worker_name: "resumed".into(),
        project: project.path().to_owned(),
        parent_session: "parent-thread".into(),
        parent_worker_id: None,
        context: WorkerContext::Resume {
            session_locator: "saved-thread".into(),
        },
        provider: None,
        model: None,
        effort: None,
        access_mode: crate::agents::HarnessAccessMode::Sandboxed,
        app_proxy: None,
        ephemeral: false,
    })?;

    assert_eq!(
        worker.poll(),
        Some(WorkerEvent::SessionChanged {
            locator: "saved-thread".into(),
        })
    );
    worker.send("after restart".into(), WorkerSendMode::Prompt)?;
    worker.close()?;

    let requests = std::fs::read_to_string(script.with_extension("sh.requests"))
        .map_err(|error| error.to_string())?;
    assert!(
        requests.contains("\"method\":\"thread/resume\""),
        "{requests}"
    );
    assert!(
        requests.contains("\"threadId\":\"saved-thread\""),
        "{requests}"
    );
    assert!(
        !requests.contains("\"method\":\"thread/fork\""),
        "{requests}"
    );
    assert!(requests.contains("after restart"), "{requests}");
    Ok(())
}

#[test]
fn delivered_inputs_preserve_images() {
    use crate::protocol::PromptImage;

    for (prefix, mode) in [
        (STEER_CLIENT_ID_PREFIX, WorkerSendMode::Steer),
        (QUEUE_CLIENT_ID_PREFIX, WorkerSendMode::Queue),
    ] {
        for text in [Some("look at these"), None] {
            let mut session = test_session();
            let mut content = vec![
                json!({"type":"image", "url":"data:image/png;base64,aGVsbG8="}),
                json!({"type":"image", "url":"data:image/jpeg;base64,d29ybGQ="}),
            ];
            if let Some(text) = text {
                content.insert(0, json!({"type":"text", "text":text}));
            }
            let item = json!({
                "type":"userMessage", "clientId":format!("{prefix}1"),
                "content":content,
            });
            for method in ["item/started", "item/completed"] {
                session
                    .queued_inbound
                    .push_back(Ok(CodexInbound::Notification {
                        method: method.into(),
                        params: json!({"threadId":"thread-1", "item":item}),
                    }));
            }
            assert_eq!(
                session.poll(),
                Some(WorkerEvent::Activity(
                    WorkerActivity::InputDeliveredWithImages {
                        mode,
                        message: text.unwrap_or_default().into(),
                        images: vec![
                            PromptImage::new("aGVsbG8=".into(), "image/png".into()),
                            PromptImage::new("d29ybGQ=".into(), "image/jpeg".into()),
                        ],
                    }
                ))
            );
            assert_eq!(session.poll(), None);
        }
    }
}

#[test]
fn skill_refresh_updates_commands_and_attaches_paths_to_prompts() {
    use std::io::BufRead as _;
    let (mut session, mut sent) = writable_test_session();
    for _ in 0..2 {
        session
            .queued_inbound
            .push_back(Ok(CodexInbound::Notification {
                method: "skills/changed".into(),
                params: json!({}),
            }));
    }
    assert!(session.poll().is_none());
    let mut requests = Vec::new();
    for _ in 0..2 {
        let mut line = String::new();
        sent.read_line(&mut line).unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "skills/list");
        assert_eq!(
            request["params"],
            json!({"cwds":["/project"], "forceReload":true})
        );
        requests.push(serde_json::from_value::<CodexRequestId>(request["id"].clone()).unwrap());
    }
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: requests[1].clone(),
        result: json!({"data":[{"cwd":"/project", "skills":[
            {"name":"review", "description":"Review code", "enabled":true, "path":"/skills/review/SKILL.md"}
        ]}]}),
    }));
    assert!(
        matches!(session.poll(), Some(WorkerEvent::Activity(WorkerActivity::CommandsChanged { commands }))
        if commands.iter().any(|command| command == &json!({"name":"skill:review", "description":"Review code", "source":"skill"})) && commands.len() == 6)
    );
    // A late reply from an older refresh must not erase the new catalog.
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: requests[0].clone(),
        result: json!({"data":[]}),
    }));
    assert!(session.poll().is_none());
    session
        .send_with_images(
            "$skill:review changes".into(),
            WorkerSendMode::Prompt,
            Vec::new(),
        )
        .unwrap();
    let mut line = String::new();
    sent.read_line(&mut line).unwrap();
    let request: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(request["method"], "turn/start");
    assert_eq!(
        request["params"]["input"],
        json!([
            {"type":"text", "text":"$review changes", "text_elements":[]},
            {"type":"skill", "name":"review", "path":"/skills/review/SKILL.md"}
        ])
    );
}

#[test]
fn native_child_events_carry_metadata_and_emit_one_finished_activity() {
    let mut session = test_session();
    session.thread_id = "native-parent".into();
    for (kind, running) in [
        ("interacted", None),
        ("started", Some(true)),
        ("interacted", Some(true)),
        ("interrupted", Some(false)),
        ("completed", Some(false)),
        ("failed", Some(false)),
        ("interacted", Some(false)),
    ] {
        let item = json!({"type": "subAgentActivity", "id": kind,
            "kind": kind, "agentThreadId": "native-event-child", "agentPath": "/root/reviewer"});
        // A notification for another parent must not affect this session.
        for (method, thread) in [
            ("item/completed", "unrelated"),
            ("item/started", "native-parent"),
            ("item/completed", "native-parent"),
        ] {
            session
                .queued_inbound
                .push_back(Ok(CodexInbound::Notification {
                    method: method.into(),
                    params: json!({"threadId": thread, "item": item}),
                }));
        }
        if kind != "interacted" {
            let outcome = match kind {
                "completed" => Some(crate::agents::ChildSessionOutcome::Complete),
                "failed" => Some(crate::agents::ChildSessionOutcome::Failed),
                "interrupted" => Some(crate::agents::ChildSessionOutcome::Incomplete),
                _ => None,
            };
            assert_eq!(
                session.poll(),
                Some(WorkerEvent::Activity(
                    WorkerActivity::ChildSessionsChanged {
                        id: "native-event-child".into(),
                        title: Some("/root/reviewer".into()),
                        is_running: running.expect("test operation should succeed"),
                        outcome,
                    }
                ))
            );
        }
        assert!(
            matches!(session.poll(), Some(WorkerEvent::Activity(WorkerActivity::ToolStarted { args, .. }))
            if args["agentThreadId"] == "native-event-child" && args["kind"] == kind)
        );
        assert!(
            matches!(session.poll(), Some(WorkerEvent::Activity(WorkerActivity::ToolFinished { is_error: false, result, .. }))
            if result[0]["text"] == format!("/root/reviewer {kind}"))
        );
        assert!(session.events.is_empty());
        assert!(session.queued_inbound.is_empty());
        assert_eq!(
            super::super::subagents::is_running("native-event-child"),
            running
        );
    }
    session.close().expect("test operation should succeed");
    assert_eq!(
        super::super::subagents::is_running("native-event-child"),
        None
    );
}

#[test]
fn interactions_read_child_turn_status_and_discard_superseded_reads() {
    use std::io::BufRead as _;

    let (mut session, mut sent) = writable_test_session();
    session.thread_id = "interaction-read-parent".into();
    let child = "interaction-read-child";
    for status in ["completed", "inProgress", "failed", "completed"] {
        session.observe_child_activity(&json!({
            "agentThreadId": child, "agentPath": "/root/reviewer", "kind": "interacted"
        }));
        let mut line = String::new();
        sent.read_line(&mut line)
            .expect("test operation should succeed");
        let request: Value = serde_json::from_str(&line).expect("test operation should succeed");
        assert_eq!(request["method"], "thread/read");
        assert_eq!(
            request["params"],
            json!({"threadId": child, "includeTurns": true})
        );
        session.queued_inbound.push_back(Ok(CodexInbound::Response {
            id: CodexRequestId::Number(session.next_id),
            result: json!({"thread": {"status": {"type": "notLoaded"}, "turns": [{"status": status}]}}),
        }));
        assert_eq!(
            session.poll(),
            Some(WorkerEvent::Activity(
                WorkerActivity::ChildSessionsChanged {
                    id: child.into(),
                    title: Some("/root/reviewer".into()),
                    is_running: status == "inProgress",
                    outcome: match status {
                        "completed" => Some(crate::agents::ChildSessionOutcome::Complete),
                        "failed" => Some(crate::agents::ChildSessionOutcome::Failed),
                        _ => None,
                    },
                }
            ))
        );
    }
    session.observe_child_activity(&json!({"agentThreadId": child, "kind": "interacted"}));
    let request = CodexRequestId::Number(session.next_id);
    session.queued_inbound.push_back(Ok(CodexInbound::Notification {
        method: "item/completed".into(),
        params: json!({"threadId": session.thread_id, "item": {
            "type": "subAgentActivity", "id": "finished", "agentThreadId": child, "kind": "completed"
        }}),
    }));
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: request,
        result: json!({"thread": {"status": {"type": "active"}}}),
    }));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::Activity(
            WorkerActivity::ChildSessionsChanged {
                is_running: false,
                ..
            }
        ))
    ));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::Activity(WorkerActivity::ToolStarted { .. }))
    ));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::Activity(WorkerActivity::ToolFinished { .. }))
    ));
    assert_eq!(session.poll(), None);
    assert_eq!(super::super::subagents::is_running(child), Some(false));
    session.close().expect("test operation should succeed");
}

fn test_session() -> CodexWorkerSession {
    use crate::modules::agents::core::{CallerProfile, CallerRegistry};

    let registry = CallerRegistry::default();
    let caller_identity = registry.issue(
        std::path::Path::new("/project"),
        CallerProfile {
            backend: "codex-cli".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    let (_sender, incoming) = mpsc::channel();
    CodexWorkerSession {
        caller_identity,
        child: std::process::Command::new("true")
            .spawn()
            .expect("test child"),
        writer: None,
        incoming,
        thread_id: "thread-1".into(),
        model: None,
        effort: None,
        collaboration_mode: None,
        collaboration_modes: HashMap::new(),
        command_state: commands::State::default(),
        skills: Skills::default(),
        project: "/project".into(),
        native_queue: false,
        next_id: 0,
        current_turn: None,
        abort_starting_turn: false,
        output: String::new(),
        reasoning_started: false,
        compacting: false,
        manual_compaction: false,
        pending: HashMap::new(),
        pending_inputs: HashMap::new(),
        prompt_requests: HashMap::new(),
        client_submissions: HashMap::new(),
        native_inputs: HashMap::new(),
        native_input_order: VecDeque::new(),
        handoff: None,
        batch_deliveries: HashMap::new(),
        normal_start_clients: HashMap::new(),
        prompt_acks: VecDeque::new(),
        acknowledged_prompts: HashSet::new(),
        queued_inbound: VecDeque::new(),
        peer_messages: VecDeque::new(),
        events: VecDeque::new(),
        turn_error: None,
    }
}

pub(super) fn writable_test_session() -> (
    CodexWorkerSession,
    std::io::BufReader<std::process::ChildStdout>,
) {
    let mut session = test_session();
    session.child.wait().expect("reap initial test child");
    let mut child = std::process::Command::new("cat")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("test echo child");
    let writer = child.stdin.take().expect("test child stdin");
    let reader = child.stdout.take().expect("test child stdout");
    session.child = child;
    session.writer = Some(writer);
    (session, std::io::BufReader::new(reader))
}

fn assert_deferred_interrupt(start: CodexInbound) {
    use std::io::BufRead as _;

    let (mut session, mut sent) = writable_test_session();
    session
        .send("work".into(), WorkerSendMode::Prompt)
        .expect("start turn");
    let mut request = String::new();
    sent.read_line(&mut request).expect("read turn start");
    assert_eq!(
        serde_json::from_str::<Value>(&request).expect("decode turn start")["method"],
        "turn/start"
    );

    session.abort().expect("defer interrupt");
    assert!(session.abort_starting_turn);
    assert_eq!(
        session.next_id, 1,
        "an unknown turn cannot be interrupted yet"
    );

    session.queued_inbound.push_back(Ok(start));
    assert!(matches!(session.poll(), Some(WorkerEvent::Started)));
    assert!(!session.abort_starting_turn);

    let mut request = String::new();
    sent.read_line(&mut request)
        .expect("read deferred interrupt");
    let interrupt = serde_json::from_str::<Value>(&request).expect("decode turn interrupt");
    assert_eq!(interrupt["method"], "turn/interrupt");
    assert_eq!(interrupt["params"]["turnId"], "turn-1");
}

#[test]
fn abort_before_turn_id_interrupts_after_start_response() {
    assert_deferred_interrupt(CodexInbound::Response {
        id: CodexRequestId::Number(1),
        result: json!({"turn": {"id": "turn-1", "status": "inProgress"}}),
    });
}

#[test]
fn abort_before_turn_id_interrupts_after_started_notification() {
    assert_deferred_interrupt(CodexInbound::Notification {
        method: "turn/started".into(),
        params: json!({"threadId": "thread-1", "turn": {"id": "turn-1"}}),
    });
}

#[test]
fn answer_before_reasoning_uses_a_separate_content_slot() {
    let mut session = test_session();

    for turn_id in ["turn-1", "turn-2"] {
        assert!(session.begin_turn(turn_id));
        assert!(!session.begin_turn(turn_id));
        assert!(matches!(
            session.poll(),
            Some(WorkerEvent::Activity(WorkerActivity::ThinkingStarted {
                content_index: 0
            }))
        ));
        for (method, delta, index) in [
            ("item/agentMessage/delta", "The answer", 1),
            ("item/reasoning/summaryTextDelta", "A thought", 0),
            ("item/agentMessage/delta", " continues", 1),
        ] {
            session
                .queued_inbound
                .push_back(Ok(CodexInbound::Notification {
                    method: method.into(),
                    params: json!({"threadId": "thread-1", "turnId": turn_id, "delta": delta}),
                }));
            match session.poll().expect("stream event") {
                WorkerEvent::Activity(WorkerActivity::TextDelta {
                    content_index,
                    delta: text,
                }) => {
                    assert_eq!(index, 1);
                    assert_eq!(content_index, index);
                    assert_eq!(text, delta);
                }
                WorkerEvent::Activity(WorkerActivity::ThinkingDelta {
                    content_index,
                    delta: text,
                }) => {
                    assert_eq!(index, 0);
                    assert_eq!(content_index, index);
                    assert_eq!(text, delta);
                }
                event => panic!("unexpected event: {event:?}"),
            }
        }
        for (method, params) in [
            (
                "item/completed",
                json!({"threadId": "thread-1", "turnId": turn_id,
                    "item": {"type": "agentMessage", "id": "final", "text": "Final answer"}}),
            ),
            (
                "turn/completed",
                json!({"threadId": "thread-1", "turn": {"id": turn_id, "status": "completed"}}),
            ),
        ] {
            session
                .queued_inbound
                .push_back(Ok(CodexInbound::Notification {
                    method: method.into(),
                    params,
                }));
        }
        assert!(matches!(
            session.poll(),
            Some(WorkerEvent::Settled { output }) if output == "Final answer"
        ));
    }
}

#[test]
fn maps_codex_telemetry() {
    assert_eq!(
        codex_telemetry(
            "mcpServer/startupStatus/updated",
            &json!({"name": "farcaster", "status": "ready"}),
        ),
        Some(WorkerActivity::ServiceStatusChanged {
            name: "farcaster".into(),
            status: "ready".into(),
            error: None,
            failure_reason: None,
        })
    );

    let limits = json!({"primary": {"usedPercent": 40}});
    assert_eq!(
        codex_telemetry(
            "account/rateLimits/updated",
            &json!({"rateLimits": limits.clone()}),
        ),
        Some(WorkerActivity::RateLimitsChanged { limits })
    );
}

#[test]
fn decodes_codex_goal_state() {
    assert_eq!(
        decode_codex_goal(&json!({
            "objective": "Ship the release",
            "status": "active",
            "tokenBudget": 200000,
            "tokensUsed": 10000,
            "timeUsedSeconds": 60
        })),
        Ok(Some(SessionGoal {
            objective: "Ship the release".into(),
            status: "active".into(),
            token_budget: Some(200000),
            tokens_used: 10000,
            time_used_seconds: 60,
        }))
    );
    assert_eq!(decode_codex_goal(&Value::Null), Ok(None));
}

#[test]
fn codex_model_efforts_accept_current_and_legacy_shapes() {
    assert_eq!(
        supported_model_efforts(&json!({
            "supportedReasoningEfforts": [
                {"reasoningEffort": "low"},
                "high"
            ]
        })),
        ["low", "high"]
    );
}

#[test]
fn extracts_completed_agent_message_text() {
    assert_eq!(
        codex_agent_message_text(&json!({
            "type": "agentMessage",
            "content": [{"type": "Text", "text": "hello"}]
        }))
        .as_deref(),
        Some("hello")
    );
}

#[test]
fn completed_tools_publish_late_metadata() {
    let completed = json!({"item": {
        "type":"fileChange",
        "id":"change-1",
        "changes":[{"path":"a.rs"},{"path":"b.rs"}],
        "status":"completed"
    }});
    assert_eq!(
        codex_tool_metadata_changed(&completed),
        Some(WorkerActivity::ToolMetadataChanged {
            id: "change-1".into(),
            args: Some(json!({
                "path":"a.rs",
                "changes":[{"path":"a.rs"},{"path":"b.rs"}]
            })),
            metadata: tool::metadata(&completed["item"], "fileChange"),
        })
    );
}

#[test]
fn automatic_approval_review_targets_the_reviewed_tool() {
    let started = json!({
        "targetItemId":"exec-1",
        "action": {
            "type":"command",
            "command":"git add logo.svg",
            "cwd":"/project"
        }
    });
    assert_eq!(
        codex_tool_review_started(&started),
        Some((
            WorkerActivity::ToolStarted {
                id: "exec-1".into(),
                name: "bash".into(),
                args: json!({"command":"git add logo.svg", "cwd":"/project"}),
                metadata: tool::metadata(&started["action"], "command"),
            },
            WorkerActivity::ToolReviewChanged {
                id: "exec-1".into(),
                state: ToolReviewState::Reviewing,
                detail: None,
            },
        ))
    );

    let completed = json!({
        "targetItemId":"exec-1",
        "review": {
            "status":"approved",
            "riskLevel":"low",
            "userAuthorization":"high",
            "rationale":"The command only stages the requested file."
        }
    });
    assert_eq!(
        codex_tool_review_completed(&completed),
        Some((
            WorkerActivity::ToolReviewChanged {
                id: "exec-1".into(),
                state: ToolReviewState::Approved,
                detail: Some(
                    "Risk: low\nAuthorization: high\nThe command only stages the requested file."
                        .into()
                ),
            },
            None,
        ))
    );
}

#[test]
fn denied_automatic_approval_review_ends_the_pending_tool() {
    assert_eq!(
        codex_tool_review_completed(&json!({
            "targetItemId":"exec-1",
            "review":{"status":"denied", "rationale":"Too broad"}
        })),
        Some((
            WorkerActivity::ToolReviewChanged {
                id: "exec-1".into(),
                state: ToolReviewState::Blocked,
                detail: Some("Too broad".into()),
            },
            Some(WorkerActivity::ToolFinished {
                id: "exec-1".into(),
                result: json!([]),
                is_error: true,
            }),
        ))
    );
}

#[test]
fn command_completion_accepts_empty_output_and_preserves_failures() {
    for status in ["completed", "failed"] {
        for output in [None, Some(Value::Null)] {
            let mut item = json!({
                "id": "exec-1",
                "type": "commandExecution",
                "status": status,
            });
            if let Some(output) = output {
                item["aggregatedOutput"] = output;
            }
            assert_eq!(
                codex_tool_end(&json!({"item": item})),
                Some(WorkerActivity::ToolFinished {
                    id: "exec-1".into(),
                    result: json!([{"type": "text", "text": ""}]),
                    is_error: status != "completed",
                })
            );
        }
    }
    assert_eq!(
        codex_tool_end(&json!({"item": {
            "id": "exec-1", "type": "commandExecution", "status": "failed",
            "aggregatedOutput": null, "error": {"message": "Permission denied"}
        }})),
        Some(WorkerActivity::ToolFinished {
            id: "exec-1".into(),
            result: json!([{"type": "text", "text": "Permission denied"}]),
            is_error: true,
        })
    );
}

#[test]
fn completed_mcp_call_preserves_structured_content() {
    assert_eq!(
        codex_tool_end(&json!({
            "item": {
                "id": "tool-1",
                "type": "mcpToolCall",
                "status": "completed",
                "result": {"content": [{"type":"text", "text":"done"}]}
            }
        })),
        Some(WorkerActivity::ToolFinished {
            id: "tool-1".into(),
            result: json!([{"type":"text", "text":"done"}]),
            is_error: false,
        })
    );
}

#[test]
fn completed_web_search_exposes_its_query_as_output() {
    let event = codex_tool_end(&json!({
        "item": {
            "id": "search-1",
            "type": "webSearch",
            "query": "Codex app-server protocol"
        }
    }));
    assert_eq!(
        event,
        Some(WorkerActivity::ToolFinished {
            id: "search-1".into(),
            result: json!([{"type":"text", "text":"Codex app-server protocol"}]),
            is_error: false,
        })
    );
}

#[test]
fn sleep_items_render_as_waiting_tools() {
    let started = json!({
        "item": {
            "type":"sleep",
            "id":"call_jmQp",
            "durationMs":50000
        }
    });
    assert_eq!(
        codex_tool_start(&started),
        Some(WorkerActivity::ToolStarted {
            id: "call_jmQp".into(),
            name: "wait".into(),
            args: json!({"durationMs": 50000}),
            metadata: tool::metadata(&started["item"], "sleep"),
        })
    );
    assert_eq!(
        codex_tool_end(&started),
        Some(WorkerActivity::ToolFinished {
            id: "call_jmQp".into(),
            result: json!([{"type": "text", "text": "Waited 50s"}]),
            is_error: false,
        })
    );
}

#[test]
fn turn_failures_carry_the_reported_codex_error() {
    let error = json!({
        "error": {
            "message": "Selected model is at capacity. Please try a different model.",
            "codexErrorInfo": "serverOverloaded"
        },
        "willRetry": false
    });
    assert_eq!(
        codex_turn_failure(codex_error_message(&error)),
        "Codex worker turn failed: Selected model is at capacity. Please try a different model."
    );
}

#[test]
fn codex_usage_separates_cached_tokens_from_reported_input() {
    assert_eq!(
        codex_usage(&json!({
            "inputTokens": 1_000,
            "outputTokens": 50,
            "cachedInputTokens": 950,
            "cacheWriteInputTokens": 0
        })),
        TokenUsage {
            input: 50,
            output: 50,
            cache_read: 950,
            cache_write: 0,
        }
    );
}

#[test]
fn native_startup_configures_required_farcaster_mcp() {
    let mut command = std::process::Command::new("codex");
    configure_codex_app_server(&mut command, crate::agents::HarnessAccessMode::Full);
    configure_farcaster_mcp(&mut command, "caller-1");
    let arguments = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        &arguments[..5],
        [
            "--dangerously-bypass-approvals-and-sandbox",
            "app-server",
            "--stdio",
            "--enable",
            "mcp_2026_07_28",
        ]
    );
    assert!(arguments.contains(&format!(
        "mcp_servers.farcaster.url=\"{}\"",
        farcaster_mcp::URL
    )));
    assert!(arguments.contains(
        &"mcp_servers.farcaster.http_headers={\"farcaster-caller\"=\"caller-1\"}".to_owned()
    ));
    assert!(arguments.contains(&"mcp_servers.farcaster.required=true".to_owned()));
}

#[test]
fn prompt_ack_requires_the_matching_rpc_reply_for_every_delivery_mode() {
    for mode in [
        WorkerSendMode::Prompt,
        WorkerSendMode::Steer,
        WorkerSendMode::Queue,
    ] {
        let (mut session, _sent) = writable_test_session();
        session.native_queue = true;
        session.current_turn = Some("active".into());
        assert!(
            !session
                .submit_prompt("submission".into(), "work".into(), mode, Vec::new())
                .unwrap()
        );
        assert!(session.poll_prompt_ack().is_none());
        let id = CodexRequestId::Number(session.next_id);
        session.queued_inbound.push_back(Ok(CodexInbound::Response {
            id: CodexRequestId::Number(9999),
            result: json!({}),
        }));
        session.poll();
        assert!(session.poll_prompt_ack().is_none());
        let result = match mode {
            WorkerSendMode::Queue => json!({"queuedSubmission": {
                "id":"queued-1","clientUserMessageId":"farcaster-queue-1","input":[]
            }}),
            WorkerSendMode::Steer => json!({"turnId":"active"}),
            WorkerSendMode::Prompt => {
                json!({"turn":{"id":"active","status":"inProgress","items":[]}})
            }
        };
        session
            .queued_inbound
            .push_back(Ok(CodexInbound::Response { id, result }));
        for _ in 0..5 {
            session.poll();
        }
        assert_eq!(
            session.poll_prompt_ack(),
            Some(("submission".into(), Ok(())))
        );
        assert!(session.poll_prompt_ack().is_none());
    }
}

#[test]
fn rejected_and_malformed_codex_replies_have_distinct_receipt_outcomes() {
    for reply in [
        json!({"error":{"code":-1,"message":"rejected"}}),
        json!({"result":{}}),
    ] {
        let (mut session, _sent) = writable_test_session();
        session
            .submit_prompt(
                "submission".into(),
                "".into(),
                WorkerSendMode::Prompt,
                Vec::new(),
            )
            .unwrap();
        let rejected = reply.get("error").is_some();
        let mut reply = reply;
        reply["id"] = json!(session.next_id);
        session
            .queued_inbound
            .push_back(super::super::wire::decode_frame(
                reply.to_string().as_bytes(),
            ));
        let events = (0..5).filter_map(|_| session.poll()).collect::<Vec<_>>();
        if rejected {
            assert!(matches!(session.poll_prompt_ack(), Some((id, Err(_))) if id == "submission"));
        } else {
            assert!(
                session.poll_prompt_ack().is_none(),
                "malformed success proves no rejection"
            );
            assert!(
                events.iter().any(|event| matches!(
                    event,
                    WorkerEvent::PromptDeliveryUnknown { submission_id, .. }
                        if submission_id == "submission"
                )),
                "malformed success must remain correlated as delivery unknown"
            );
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, WorkerEvent::Failed(_))),
                "malformed success is request-local"
            );
        }
    }
}

#[test]
fn rejected_steer_and_interrupt_requests_do_not_fail_the_session() {
    let (mut session, _sent) = writable_test_session();
    session.current_turn = Some("active".into());
    session
        .submit_prompt(
            "steer-submission".into(),
            "redirect".into(),
            WorkerSendMode::Steer,
            Vec::new(),
        )
        .expect("send steer");
    let steer_id = CodexRequestId::Number(session.next_id);
    session.queued_inbound.push_back(Ok(CodexInbound::Error {
        id: steer_id,
        error: super::super::contract::CodexRpcError {
            code: -32000,
            message: "turn is no longer active".into(),
            data: Value::Null,
        },
    }));
    assert!(session.poll().is_none());
    assert!(matches!(session.poll_prompt_ack(), Some((id, Err(_))) if id == "steer-submission"));

    session.interrupt_turn("active").expect("send interrupt");
    let interrupt_id = CodexRequestId::Number(session.next_id);
    session.queued_inbound.push_back(Ok(CodexInbound::Error {
        id: interrupt_id,
        error: super::super::contract::CodexRpcError {
            code: -32000,
            message: "turn is no longer active".into(),
            data: Value::Null,
        },
    }));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::RequestFailed { operation, error })
            if operation == "Codex interrupt" && error == "turn is no longer active"
    ));

    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "item/agentMessage/delta".into(),
            params: json!({"threadId":"thread-1","turnId":"active","delta":"still alive"}),
        }));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::Activity(WorkerActivity::TextDelta { delta, .. }))
            if delta == "still alive"
    ));
}

#[test]
fn correlated_codex_control_rejection_reaches_the_session_caller() {
    use crate::agents::extensions::PromptMode;
    use crate::agents::{SessionCommand, SessionEvent, SessionOperation, SessionTransport};
    use crate::modules::agents::adapter::main_session::{
        MainSessionMetadata, WorkerSessionTransport,
    };

    let (mut session, _sent) = writable_test_session();
    let (incoming, receiver) = mpsc::channel();
    session.incoming = receiver;
    session.current_turn = Some("active".into());
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "codex-cli",
        "thread-1".into(),
        Box::new(session),
        MainSessionMetadata::default(),
        None,
    )
    .expect("Codex transport");
    let submission_id = transport
        .send(SessionCommand::Prompt {
            mode: PromptMode::Steer,
            message: "redirect".into(),
            images: Vec::new(),
        })
        .expect("send steer");
    incoming
        .send(Ok(CodexInbound::Error {
            id: CodexRequestId::Number(1),
            error: super::super::contract::CodexRpcError {
                code: -32000,
                message: "turn is no longer active".into(),
                data: Value::Null,
            },
        }))
        .expect("steer rejection");

    let response = loop {
        if let Some(SessionEvent::Response(response)) = transport.poll() {
            break response;
        }
    };
    assert_eq!(response.id.as_deref(), Some(submission_id.as_str()));
    let error = response.result.expect_err("steer must be rejected");
    assert_eq!(error.message, "turn is no longer active");
    assert_eq!(error.operation, SessionOperation::Prompt(PromptMode::Steer));
}

#[test]
fn malformed_success_is_delivery_unknown_for_every_prompt_mode() {
    use crate::agents::extensions::PromptMode;
    use crate::agents::{SessionCommand, SessionEvent, SessionResponseErrorKind, SessionTransport};
    use crate::app::views::transcript::conversation::{ConversationState, TranscriptKind};
    use crate::modules::agents::adapter::main_session::{
        MainSessionMetadata, WorkerSessionTransport,
    };

    fn project(
        transport: &mut WorkerSessionTransport,
        conversation: &mut ConversationState,
        responses: &mut Vec<crate::agents::SessionResponse>,
        failures: &mut Vec<String>,
    ) {
        while let Some(event) = transport.poll() {
            match event {
                SessionEvent::Activity(activity) => {
                    conversation.reduce(activity.value());
                }
                SessionEvent::Response(response) => responses.push(response),
                SessionEvent::Failure(error) => failures.push(error),
                other => panic!("unexpected event: {other:?}"),
            }
        }
    }

    for mode in [PromptMode::Normal, PromptMode::Steer, PromptMode::FollowUp] {
        let (mut session, _sent) = writable_test_session();
        let (incoming, receiver) = mpsc::channel();
        session.incoming = receiver;
        session.native_queue = true;
        if mode != PromptMode::Normal {
            session.current_turn = Some("turn-1".into());
        }
        let mut transport = WorkerSessionTransport::new(
            std::path::Path::new("/locators"),
            "codex-cli",
            "thread-1".into(),
            Box::new(session),
            MainSessionMetadata::default(),
            None,
        )
        .expect("Codex transport");
        assert!(
            transport.tracks_prompt_delivery(mode),
            "Codex receipts must stay eligible for unresolved history recovery"
        );
        let submission_id = transport
            .send(SessionCommand::Prompt {
                mode,
                message: "retain unknown".into(),
                images: vec![crate::protocol::PromptImage::new(
                    "AQID".into(),
                    "image/png".into(),
                )],
            })
            .expect("submit prompt");
        incoming
            .send(Ok(CodexInbound::Response {
                id: CodexRequestId::Number(1),
                result: json!({}),
            }))
            .expect("malformed success");

        let mut conversation = ConversationState::default();
        let mut responses = Vec::new();
        let mut failures = Vec::new();
        project(
            &mut transport,
            &mut conversation,
            &mut responses,
            &mut failures,
        );
        let response = responses
            .iter()
            .find(|response| response.id.as_deref() == Some(submission_id.as_str()))
            .expect("prompt outcome");
        assert_eq!(
            response
                .result
                .as_ref()
                .expect_err("malformed success is unknown")
                .kind,
            SessionResponseErrorKind::DeliveryUnknown
        );
        assert!(!responses.iter().any(|response| {
            response.result.as_ref().is_err_and(|error| {
                error.kind == SessionResponseErrorKind::RejectedBeforeAcceptance
            })
        }));
        let users = conversation
            .items
            .iter()
            .filter(|item| item.kind == TranscriptKind::User)
            .collect::<Vec<_>>();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].text, "retain unknown");
        assert_eq!(users[0].images.len(), 1);
        assert_eq!(users[0].label, "Delivery unknown");

        let client_id = match mode {
            PromptMode::Normal => "farcaster-normal-1",
            PromptMode::Steer => "farcaster-steer-1",
            PromptMode::FollowUp => "farcaster-queue-1",
        };
        incoming
            .send(Ok(CodexInbound::Notification {
                method: "item/started".into(),
                params: json!({"threadId":"thread-1","turnId":"turn-1","item":{
                    "type":"userMessage","clientId":client_id,"content":[
                        {"type":"text","text":"retain unknown"},
                        {"type":"image","url":"data:image/png;base64,AQID"}
                    ]
                }}),
            }))
            .expect("late old delivery");
        project(
            &mut transport,
            &mut conversation,
            &mut responses,
            &mut failures,
        );
        let users = conversation
            .items
            .iter()
            .filter(|item| item.kind == TranscriptKind::User)
            .collect::<Vec<_>>();
        assert_eq!(users.len(), 1, "late evidence must reconcile, not replay");
        assert_eq!(users[0].text, "retain unknown");
        assert_eq!(users[0].images.len(), 1);
        assert!(users[0].label.is_empty());

        let later_id = transport
            .send(SessionCommand::Prompt {
                mode,
                message: "later prompt".into(),
                images: Vec::new(),
            })
            .expect("later prompt remains live");
        let result = match mode {
            PromptMode::Normal => {
                json!({"turn":{"id":"turn-2","status":"inProgress"}})
            }
            PromptMode::Steer => json!({"turnId":"turn-1"}),
            PromptMode::FollowUp => json!({"queuedSubmission": {
                "id":"queued-2","clientUserMessageId":"farcaster-queue-2","input":[]
            }}),
        };
        incoming
            .send(Ok(CodexInbound::Response {
                id: CodexRequestId::Number(2),
                result,
            }))
            .expect("later prompt response");
        project(
            &mut transport,
            &mut conversation,
            &mut responses,
            &mut failures,
        );
        assert!(responses.iter().any(|response| {
            response.id.as_deref() == Some(later_id.as_str()) && response.result.is_ok()
        }));
        assert!(failures.is_empty(), "malformed success is request-local");
    }
}

#[test]
fn malformed_native_reply_stays_correlatable_but_never_joins_next_handoff() {
    use std::io::BufRead as _;

    let (mut session, mut sent) = writable_test_session();
    session.current_turn = Some("turn-1".into());
    session
        .submit_prompt(
            "old".into(),
            "old unknown".into(),
            WorkerSendMode::Steer,
            vec![crate::protocol::PromptImage::new(
                "AQID".into(),
                "image/png".into(),
            )],
        )
        .expect("old steer");
    let mut line = String::new();
    sent.read_line(&mut line).expect("old steer request");
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: CodexRequestId::Number(1),
        result: json!({}),
    }));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::PromptDeliveryUnknown { submission_id, .. }) if submission_id == "old"
    ));

    session
        .submit_prompt(
            "new".into(),
            "new accepted".into(),
            WorkerSendMode::Steer,
            Vec::new(),
        )
        .expect("new steer");
    line.clear();
    sent.read_line(&mut line).expect("new steer request");
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: CodexRequestId::Number(2),
        result: json!({"turnId":"turn-1"}),
    }));
    assert!(session.poll().is_none());
    session.apply_steering().expect("apply new steer");
    line.clear();
    sent.read_line(&mut line).expect("interrupt request");
    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-1","turn":{
                "id":"turn-1","status":"interrupted"
            }}),
        }));
    let _ = session.poll();
    line.clear();
    sent.read_line(&mut line).expect("new handoff batch");
    let batch: Value = serde_json::from_str(&line).expect("decode new batch");
    let text = batch["params"]["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|part| part["text"].as_str())
        .collect::<String>();
    assert_eq!(text, "new accepted");

    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "item/started".into(),
            params: json!({"threadId":"thread-1","turnId":"turn-1","item":{
                "type":"userMessage","clientId":"farcaster-steer-1","content":[
                    {"type":"text","text":"old unknown"},
                    {"type":"image","url":"data:image/png;base64,AQID"}
                ]
            }}),
        }));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::Activity(
            WorkerActivity::SubmittedInputDeliveredWithImages {
                submission_id,
                message,
                images,
                ..
            }
        )) if submission_id == "old" && message == "old unknown" && images.len() == 1
    ));
    assert_eq!(
        session
            .handoff
            .as_ref()
            .and_then(|handoff| handoff.batch_client_id.as_deref()),
        Some("farcaster-handoff-4")
    );
}

#[test]
fn late_old_unknown_batch_delivery_does_not_clear_new_handoff() {
    use std::io::BufRead as _;

    let (mut session, mut sent) = writable_test_session();
    session.current_turn = Some("turn-1".into());
    session
        .submit_prompt(
            "old".into(),
            "old batch".into(),
            WorkerSendMode::Steer,
            Vec::new(),
        )
        .expect("old steer");
    let mut line = String::new();
    sent.read_line(&mut line).expect("old steer request");
    session.apply_steering().expect("old apply");
    line.clear();
    sent.read_line(&mut line).expect("old interrupt");
    session.queued_inbound.push_back(Ok(CodexInbound::Error {
        id: CodexRequestId::Number(1),
        error: super::super::contract::CodexRpcError {
            code: -32000,
            message: "no active turn to steer".into(),
            data: Value::Null,
        },
    }));
    let _ = session.poll();
    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-1","turn":{
                "id":"turn-1","status":"interrupted"
            }}),
        }));
    let _ = session.poll();
    line.clear();
    sent.read_line(&mut line).expect("old batch request");
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: CodexRequestId::Number(3),
        result: json!({}),
    }));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::PromptDeliveryUnknown { submission_id, .. }) if submission_id == "old"
    ));
    assert!(session.handoff.is_none());

    session.current_turn = Some("turn-new".into());
    session
        .submit_prompt(
            "new".into(),
            "new batch".into(),
            WorkerSendMode::Steer,
            Vec::new(),
        )
        .expect("new steer");
    line.clear();
    sent.read_line(&mut line).expect("new steer request");
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: CodexRequestId::Number(4),
        result: json!({"turnId":"turn-new"}),
    }));
    let _ = session.poll();
    session.apply_steering().expect("new apply");
    line.clear();
    sent.read_line(&mut line).expect("new interrupt");
    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-1","turn":{
                "id":"turn-new","status":"interrupted"
            }}),
        }));
    let _ = session.poll();
    line.clear();
    sent.read_line(&mut line).expect("new batch request");
    assert_eq!(
        session
            .handoff
            .as_ref()
            .and_then(|handoff| handoff.batch_client_id.as_deref()),
        Some("farcaster-handoff-6")
    );

    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "item/started".into(),
            params: json!({"threadId":"thread-1","turnId":"old-turn","item":{
                "type":"userMessage","clientId":"farcaster-handoff-3",
                "content":[{"type":"text","text":"old batch"}]
            }}),
        }));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::Activity(WorkerActivity::SubmittedInputDelivered {
            submission_id,
            ..
        })) if submission_id == "old"
    ));
    assert_eq!(
        session
            .handoff
            .as_ref()
            .and_then(|handoff| handoff.batch_client_id.as_deref()),
        Some("farcaster-handoff-6"),
        "old delivery cannot clear the new handoff"
    );
}

#[test]
fn old_unknown_batch_does_not_block_cancelled_handoff_cleanup_or_later_apply() {
    use std::io::BufRead as _;

    let (mut session, mut sent) = writable_test_session();
    session.current_turn = Some("turn-1".into());
    session
        .submit_prompt(
            "old".into(),
            "old batch".into(),
            WorkerSendMode::Steer,
            Vec::new(),
        )
        .expect("old steer");
    let mut line = String::new();
    sent.read_line(&mut line).expect("old steer request");
    session.apply_steering().expect("old apply");
    line.clear();
    sent.read_line(&mut line).expect("old interrupt");
    session.queued_inbound.push_back(Ok(CodexInbound::Error {
        id: CodexRequestId::Number(1),
        error: super::super::contract::CodexRpcError {
            code: -32000,
            message: "no active turn to steer".into(),
            data: Value::Null,
        },
    }));
    let _ = session.poll();
    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-1","turn":{
                "id":"turn-1","status":"interrupted"
            }}),
        }));
    let _ = session.poll();
    line.clear();
    sent.read_line(&mut line).expect("old batch request");
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: CodexRequestId::Number(3),
        result: json!({}),
    }));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::PromptDeliveryUnknown { submission_id, .. }) if submission_id == "old"
    ));
    assert!(session.batch_deliveries.contains_key("farcaster-handoff-3"));

    session.current_turn = Some("turn-2".into());
    session
        .submit_prompt(
            "cancelled".into(),
            "cancel this handoff".into(),
            WorkerSendMode::Steer,
            Vec::new(),
        )
        .expect("new steer");
    line.clear();
    sent.read_line(&mut line).expect("new steer request");
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: CodexRequestId::Number(4),
        result: json!({"turnId":"turn-2"}),
    }));
    let _ = session.poll();
    session.apply_steering().expect("new apply");
    line.clear();
    sent.read_line(&mut line).expect("new interrupt");
    let apply_interrupt: Value = serde_json::from_str(&line).expect("decode new interrupt");
    assert_eq!(apply_interrupt["method"], "turn/interrupt");
    let before_abort = session.next_id;
    session.abort().expect("second escape");
    if session.next_id != before_abort {
        line.clear();
        sent.read_line(&mut line).expect("abort request");
        let abort: Value = serde_json::from_str(&line).expect("decode abort request");
        assert_eq!(abort["method"], "turn/interrupt");
        assert_eq!(abort["params"]["turnId"], "turn-2");
    }
    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-1","turn":{
                "id":"turn-2","status":"interrupted"
            }}),
        }));
    let _ = session.poll();
    assert!(session.handoff.is_none());
    assert!(session.batch_deliveries.contains_key("farcaster-handoff-3"));

    session.current_turn = Some("turn-3".into());
    session
        .submit_prompt(
            "later".into(),
            "later steer".into(),
            WorkerSendMode::Steer,
            Vec::new(),
        )
        .expect("later steer");
    line.clear();
    sent.read_line(&mut line).expect("later steer request");
    let later_request: Value = serde_json::from_str(&line).expect("decode later steer request");
    let later_request_id: CodexRequestId =
        serde_json::from_value(later_request["id"].clone()).expect("later steer request id");
    assert_eq!(later_request["method"], "turn/steer");
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: later_request_id,
        result: json!({"turnId":"turn-3"}),
    }));
    let _ = session.poll();
    session.apply_steering().expect("later apply");
    line.clear();
    sent.read_line(&mut line).expect("later interrupt");
    let interrupt: Value = serde_json::from_str(&line).expect("decode later interrupt");
    assert_eq!(interrupt["method"], "turn/interrupt");
    assert_eq!(interrupt["params"]["turnId"], "turn-3");
}

#[test]
fn native_queue_delivery_correlates_duplicate_text_before_other_rejection() {
    let (mut session, _sent) = writable_test_session();
    session.native_queue = true;
    session.current_turn = Some("active".into());
    session
        .submit_prompt(
            "first-submission".into(),
            "same text".into(),
            WorkerSendMode::Queue,
            Vec::new(),
        )
        .expect("submit first queue item");
    session
        .submit_prompt(
            "second-submission".into(),
            "same text".into(),
            WorkerSendMode::Queue,
            Vec::new(),
        )
        .expect("submit second queue item");

    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "item/started".into(),
            params: json!({"threadId":"thread-1","item":{
                "type":"userMessage",
                "clientId":"farcaster-queue-2",
                "content":[{"type":"text","text":"same text"}]
            }}),
        }));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::Activity(WorkerActivity::SubmittedInputDelivered {
            submission_id,
            mode: WorkerSendMode::Queue,
            message,
        })) if submission_id == "second-submission" && message == "same text"
    ));
    assert_eq!(
        session
            .client_submissions
            .get("farcaster-queue-1")
            .map(String::as_str),
        Some("first-submission")
    );
    assert!(!session.client_submissions.contains_key("farcaster-queue-2"));

    session.queued_inbound.push_back(Ok(CodexInbound::Error {
        id: CodexRequestId::Number(1),
        error: super::super::contract::CodexRpcError {
            code: -32000,
            message: "queue rejected".into(),
            data: Value::Null,
        },
    }));
    assert!(session.poll().is_none());
    assert_eq!(
        session.poll_prompt_ack(),
        Some(("first-submission".into(), Err("queue rejected".into())))
    );
    assert!(!session.client_submissions.contains_key("farcaster-queue-1"));
}

#[test]
fn native_queue_stays_visible_across_turn_completion_until_delivery() {
    use crate::agents::extensions::PromptMode;
    use crate::agents::{SessionCommand, SessionEvent, SessionTransport};
    use crate::app::views::transcript::conversation::ConversationState;
    use crate::modules::agents::adapter::main_session::{
        MainSessionMetadata, WorkerSessionTransport,
    };

    let (mut session, _sent) = writable_test_session();
    let (incoming, receiver) = mpsc::channel();
    session.incoming = receiver;
    session.native_queue = true;
    session.current_turn = Some("turn-1".into());
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "codex-cli",
        "thread-1".into(),
        Box::new(session),
        MainSessionMetadata::default(),
        None,
    )
    .expect("Codex transport");
    fn drain(transport: &mut WorkerSessionTransport, conversation: &mut ConversationState) {
        while let Some(event) = transport.poll() {
            if let SessionEvent::Activity(activity) = event {
                conversation.reduce(activity.value());
            }
        }
    }
    let mut conversation = ConversationState::default();

    transport
        .send(SessionCommand::Prompt {
            mode: PromptMode::FollowUp,
            message: "next task".into(),
            images: Vec::new(),
        })
        .expect("queue prompt");
    incoming
        .send(Ok(CodexInbound::Response {
            id: CodexRequestId::Number(1),
            result: json!({"queuedSubmission": {
                "id":"queued-1","clientUserMessageId":"farcaster-queue-1","input":[]
            }}),
        }))
        .expect("queue admission response");
    drain(&mut transport, &mut conversation);
    assert_eq!(conversation.queue.follow_up, ["next task"]);

    incoming
        .send(Ok(CodexInbound::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-1","turn":{"id":"turn-1","status":"completed"}}),
        }))
        .expect("turn completion");
    drain(&mut transport, &mut conversation);
    assert_eq!(conversation.queue.follow_up, ["next task"]);

    incoming
        .send(Ok(CodexInbound::Notification {
            method: "item/started".into(),
            params: json!({"threadId":"thread-1","item":{
                "type":"userMessage",
                "clientId":"farcaster-queue-1",
                "content":[{"type":"text","text":"next task"}]
            }}),
        }))
        .expect("queue delivery");
    drain(&mut transport, &mut conversation);
    assert!(conversation.queue.follow_up.is_empty());
}

#[test]
fn apply_steering_claims_all_queued_inputs_and_fans_out_batch_delivery() {
    fn read_request(reader: &mut impl std::io::BufRead) -> Value {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read Codex request");
        serde_json::from_str(&line).expect("decode Codex request")
    }

    let (mut session, mut sent) = writable_test_session();
    session.native_queue = true;
    session.current_turn = Some("turn-1".into());
    for (id, message, mode) in [
        ("steer-1", "steer now", WorkerSendMode::Steer),
        ("queue-1", "then one", WorkerSendMode::Queue),
        ("queue-2", "then two", WorkerSendMode::Queue),
    ] {
        session
            .submit_prompt(id.into(), message.into(), mode, Vec::new())
            .expect("submit pending input");
        let _ = read_request(&mut sent);
    }

    session.apply_steering().expect("apply pending input");
    assert_eq!(read_request(&mut sent)["method"], "turn/interrupt");
    for (id, queue_id) in [(2, "queued-1"), (3, "queued-2")] {
        session.queued_inbound.push_back(Ok(CodexInbound::Response {
            id: CodexRequestId::Number(id),
            result: json!({"queuedSubmission": {
                "id": queue_id,
                "clientUserMessageId": format!("farcaster-queue-{id}"),
                "input": []
            }}),
        }));
    }
    session.queued_inbound.push_back(Ok(CodexInbound::Error {
        id: CodexRequestId::Number(1),
        error: super::super::contract::CodexRpcError {
            code: -32000,
            message: "no active turn to steer".into(),
            data: Value::Null,
        },
    }));
    while session.poll().is_some() {}
    assert!(matches!(session.poll_prompt_ack(), Some((id, Ok(()))) if id == "queue-1"));
    assert!(matches!(session.poll_prompt_ack(), Some((id, Ok(()))) if id == "queue-2"));
    assert_eq!(
        session.poll_prompt_ack(),
        None,
        "steer rejection is retried"
    );

    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-1","turn":{
                "id":"stale-turn","status":"interrupted"
            }}),
        }));
    assert!(session.poll().is_none());
    assert_eq!(session.current_turn.as_deref(), Some("turn-1"));
    assert_eq!(session.next_id, 4, "stale completion cannot start claims");

    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-1","turn":{
                "id":"turn-1","status":"interrupted"
            }}),
        }));
    assert!(matches!(session.poll(), Some(WorkerEvent::Settled { .. })));
    let first_delete = read_request(&mut sent);
    let second_delete = read_request(&mut sent);
    assert_eq!(first_delete["method"], "thread/queue/delete");
    assert_eq!(second_delete["method"], "thread/queue/delete");
    assert_eq!(
        [
            first_delete["params"]["queuedSubmissionId"]
                .as_str()
                .unwrap(),
            second_delete["params"]["queuedSubmissionId"]
                .as_str()
                .unwrap(),
        ],
        ["queued-1", "queued-2"]
    );

    for id in [5, 6] {
        session.queued_inbound.push_back(Ok(CodexInbound::Response {
            id: CodexRequestId::Number(id),
            result: json!({"deleted": true}),
        }));
        let _ = session.poll();
    }
    let batch = read_request(&mut sent);
    assert_eq!(batch["method"], "turn/start");
    let batch_client_id = batch["params"]["clientUserMessageId"]
        .as_str()
        .expect("batch client id")
        .to_owned();
    let batch_text = batch["params"]["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("");
    assert_eq!(batch_text, "steer now\n\nthen one\n\nthen two");

    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: CodexRequestId::Number(7),
        result: json!({"turn":{"id":"turn-2","status":"inProgress"}}),
    }));
    let _ = session.poll();
    assert!(matches!(session.poll_prompt_ack(), Some((id, Ok(()))) if id == "steer-1"));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::Activity(
            WorkerActivity::ThinkingStarted { .. }
        ))
    ));
    assert!(matches!(session.poll(), Some(WorkerEvent::Started)));
    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "item/started".into(),
            params: json!({"threadId":"thread-1","turnId":"turn-2","item":{
                "type":"userMessage","clientId":batch_client_id,
                "content":[{"type":"text","text":batch_text}]
            }}),
        }));
    let mut delivered = Vec::new();
    for _ in 0..3 {
        match session.poll().expect("original delivery") {
            WorkerEvent::Activity(WorkerActivity::SubmittedInputDelivered {
                submission_id,
                ..
            }) => delivered.push(submission_id),
            event => panic!("unexpected event: {event:?}"),
        }
    }
    assert_eq!(delivered, ["steer-1", "queue-1", "queue-2"]);
}

#[test]
fn abort_cancels_handoff_but_acknowledges_and_deletes_late_queue_add() {
    use std::io::BufRead as _;

    let (mut session, mut sent) = writable_test_session();
    session.native_queue = true;
    session.current_turn = Some("turn-1".into());
    session
        .submit_prompt(
            "queue-1".into(),
            "do not replay".into(),
            WorkerSendMode::Queue,
            Vec::new(),
        )
        .expect("queue input");
    let mut line = String::new();
    sent.read_line(&mut line).expect("queue add");
    session.apply_steering().expect("first escape");
    line.clear();
    sent.read_line(&mut line).expect("handoff interrupt");
    session.abort().expect("second escape");

    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: CodexRequestId::Number(1),
        result: json!({"queuedSubmission": {
            "id":"queued-1","clientUserMessageId":"farcaster-queue-1","input":[]
        }}),
    }));
    assert!(session.poll().is_none());
    assert!(matches!(session.poll_prompt_ack(), Some((id, Ok(()))) if id == "queue-1"));
    line.clear();
    sent.read_line(&mut line).expect("late queue delete");
    let delete: Value = serde_json::from_str(&line).expect("decode delete");
    assert_eq!(delete["method"], "thread/queue/delete");
    assert_eq!(delete["params"]["queuedSubmissionId"], "queued-1");
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: CodexRequestId::Number(3),
        result: json!({"deleted":true}),
    }));
    assert!(session.poll().is_none());
    assert!(
        session.handoff.is_none(),
        "completed cancel must release handoff"
    );
}

#[test]
fn failed_queue_claim_waits_for_auto_started_turn_and_steers_only_safe_remainder() {
    use std::io::BufRead as _;

    let (mut session, mut sent) = writable_test_session();
    session.native_queue = true;
    session.current_turn = Some("turn-1".into());
    session
        .submit_prompt(
            "steer-1".into(),
            "safe steer".into(),
            WorkerSendMode::Steer,
            Vec::new(),
        )
        .expect("steer input");
    session
        .submit_prompt(
            "queue-1".into(),
            "may auto start".into(),
            WorkerSendMode::Queue,
            Vec::new(),
        )
        .expect("queue input");
    for _ in 0..2 {
        let mut line = String::new();
        sent.read_line(&mut line).expect("initial request");
    }
    session.apply_steering().expect("first escape");
    let mut line = String::new();
    sent.read_line(&mut line).expect("interrupt request");
    for (id, result) in [
        (1, json!({"turnId":"turn-1"})),
        (
            2,
            json!({"queuedSubmission": {
                "id":"queued-1","clientUserMessageId":"farcaster-queue-2","input":[]
            }}),
        ),
    ] {
        session.queued_inbound.push_back(Ok(CodexInbound::Response {
            id: CodexRequestId::Number(id),
            result,
        }));
    }
    while session.poll().is_some() {}
    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-1","turn":{
                "id":"turn-1","status":"interrupted"
            }}),
        }));
    let _ = session.poll();
    line.clear();
    sent.read_line(&mut line).expect("queue claim");
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["method"],
        "thread/queue/delete"
    );
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: CodexRequestId::Number(4),
        result: json!({"deleted":false}),
    }));
    assert!(session.poll().is_none());
    assert_eq!(
        session.next_id, 4,
        "do not start while ownership is uncertain"
    );

    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "turn/started".into(),
            params: json!({"threadId":"thread-1","turn":{"id":"auto-turn"}}),
        }));
    assert!(matches!(session.poll(), Some(WorkerEvent::Started)));
    line.clear();
    sent.read_line(&mut line).expect("safe remainder steer");
    let steer: Value = serde_json::from_str(&line).expect("decode remainder steer");
    assert_eq!(steer["method"], "turn/steer");
    assert_eq!(steer["params"]["expectedTurnId"], "auto-turn");
    let text = steer["params"]["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|part| part["text"].as_str())
        .collect::<String>();
    assert_eq!(text, "safe steer");
}

#[test]
fn second_abort_while_turn_starts_cancels_handoff_and_interrupts_known_turn() {
    use std::io::BufRead as _;

    let (mut session, mut sent) = writable_test_session();
    session.native_queue = true;
    session
        .send("starting work".into(), WorkerSendMode::Prompt)
        .expect("start prompt");
    session
        .submit_prompt(
            "queue-1".into(),
            "pending follow-up".into(),
            WorkerSendMode::Queue,
            Vec::new(),
        )
        .expect("queue while starting");
    for _ in 0..2 {
        let mut line = String::new();
        sent.read_line(&mut line).expect("initial request");
    }
    session.apply_steering().expect("first escape");
    assert!(session.abort_starting_turn);
    assert_eq!(session.next_id, 2, "wait for the starting turn id");
    session.abort().expect("second escape");
    assert!(
        session
            .handoff
            .as_ref()
            .is_some_and(|state| state.cancelled)
    );

    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: CodexRequestId::Number(2),
        result: json!({"queuedSubmission": {
            "id":"queued-1","clientUserMessageId":"farcaster-queue-2","input":[]
        }}),
    }));
    assert!(session.poll().is_none());
    assert!(matches!(session.poll_prompt_ack(), Some((id, Ok(()))) if id == "queue-1"));
    let mut line = String::new();
    sent.read_line(&mut line).expect("cancel queued input");
    let delete: Value = serde_json::from_str(&line).expect("decode queue delete");
    assert_eq!(delete["method"], "thread/queue/delete");

    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: CodexRequestId::Number(1),
        result: json!({"turn":{"id":"turn-starting","status":"inProgress"}}),
    }));
    assert!(matches!(session.poll(), Some(WorkerEvent::Started)));
    line.clear();
    sent.read_line(&mut line).expect("interrupt starting turn");
    let interrupt: Value = serde_json::from_str(&line).expect("decode turn interrupt");
    assert_eq!(interrupt["method"], "turn/interrupt");
    assert_eq!(interrupt["params"]["turnId"], "turn-starting");
}

fn assert_second_abort_cancels_auto_started_queue(abort_before_delete_reply: bool) {
    use std::io::BufRead as _;

    let (mut session, mut sent) = writable_test_session();
    session.native_queue = true;
    session.current_turn = Some("turn-1".into());
    session
        .submit_prompt(
            "queue-1".into(),
            "queued input".into(),
            WorkerSendMode::Queue,
            Vec::new(),
        )
        .expect("queue input");
    let mut line = String::new();
    sent.read_line(&mut line).expect("queue add");
    session.apply_steering().expect("first escape");
    line.clear();
    sent.read_line(&mut line).expect("first interrupt");
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: CodexRequestId::Number(1),
        result: json!({"queuedSubmission": {
            "id":"queued-1","clientUserMessageId":"farcaster-queue-1","input":[]
        }}),
    }));
    let _ = session.poll();
    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-1","turn":{
                "id":"turn-1","status":"interrupted"
            }}),
        }));
    let _ = session.poll();
    line.clear();
    sent.read_line(&mut line).expect("queue claim");
    if abort_before_delete_reply {
        session.abort().expect("second escape before delete reply");
    }
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: CodexRequestId::Number(3),
        result: json!({"deleted":false}),
    }));
    assert!(session.poll().is_none());
    if !abort_before_delete_reply {
        session.abort().expect("second escape after delete reply");
    }

    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "turn/started".into(),
            params: json!({"threadId":"thread-1","turn":{"id":"auto-turn"}}),
        }));
    assert!(matches!(session.poll(), Some(WorkerEvent::Started)));
    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "item/started".into(),
            params: json!({"threadId":"thread-1","turnId":"auto-turn","item":{
                "type":"userMessage","clientId":"farcaster-queue-1",
                "content":[{"type":"text","text":"queued input"}]
            }}),
        }));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::Activity(
            WorkerActivity::ThinkingStarted { .. }
        ))
    ));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::Activity(WorkerActivity::SubmittedInputDelivered {
            submission_id,
            ..
        })) if submission_id == "queue-1"
    ));
    line.clear();
    sent.read_line(&mut line)
        .expect("interrupt owned auto-start");
    let interrupt: Value = serde_json::from_str(&line).expect("decode auto-start interrupt");
    assert_eq!(interrupt["method"], "turn/interrupt");
    assert_eq!(interrupt["params"]["turnId"], "auto-turn");
    assert!(session.handoff.is_none(), "cancelled handoff must release");
}

#[test]
fn second_abort_before_failed_delete_cancels_exact_auto_started_queue() {
    assert_second_abort_cancels_auto_started_queue(true);
}

#[test]
fn second_abort_after_failed_delete_cancels_exact_auto_started_queue() {
    assert_second_abort_cancels_auto_started_queue(false);
}

#[test]
fn prompt_write_failure_is_delivery_unknown_not_local_rejection() {
    let mut session = test_session();
    session.current_turn = Some("turn-1".into());
    assert_eq!(
        session.submit_prompt(
            "steer-1".into(),
            "possibly written".into(),
            WorkerSendMode::Steer,
            Vec::new(),
        ),
        Ok(false)
    );
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::Failed(error)) if error.contains("prompt delivery is unknown")
    ));
    assert_eq!(session.poll_prompt_ack(), None);
    assert_eq!(
        session
            .prompt_requests
            .get(&CodexRequestId::Number(1))
            .map(String::as_str),
        Some("steer-1")
    );
}

#[test]
fn normal_prompt_delivery_uses_backend_client_id_and_original_submission_id() {
    use std::io::BufRead as _;

    let (mut session, mut sent) = writable_test_session();
    session
        .submit_prompt(
            "normal-1".into(),
            "new work".into(),
            WorkerSendMode::Prompt,
            Vec::new(),
        )
        .expect("submit normal prompt");
    let mut line = String::new();
    sent.read_line(&mut line).expect("turn start");
    let request: Value = serde_json::from_str(&line).expect("decode turn start");
    assert_eq!(
        request["params"]["clientUserMessageId"],
        "farcaster-normal-1"
    );
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: CodexRequestId::Number(1),
        result: json!({"turn":{"id":"turn-1","status":"inProgress"}}),
    }));
    assert!(matches!(session.poll(), Some(WorkerEvent::Started)));
    assert!(matches!(session.poll_prompt_ack(), Some((id, Ok(()))) if id == "normal-1"));
    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "item/started".into(),
            params: json!({"threadId":"thread-1","turnId":"turn-1","item":{
                "type":"userMessage","clientId":"farcaster-normal-1",
                "content":[{"type":"text","text":"new work"}]
            }}),
        }));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::Activity(
            WorkerActivity::ThinkingStarted { .. }
        ))
    ));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::Activity(WorkerActivity::SubmittedInputDelivered {
            submission_id,
            mode: WorkerSendMode::Prompt,
            message,
        })) if submission_id == "normal-1" && message == "new work"
    ));
}

#[test]
fn process_fixture_interrupts_claims_and_delivers_one_native_batch() {
    const SCRIPT: &str = r#"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"turn/steer"'*)
      printf '{"id":%s,"error":{"code":-32000,"message":"no active turn to steer","data":null}}\n' "$id"
      ;;
    *'"method":"thread/queue/add"'*)
      printf '{"id":%s,"result":{"queuedSubmission":{"id":"queued-1","clientUserMessageId":"farcaster-queue-2","input":[]}}}\n' "$id"
      ;;
    *'"method":"turn/interrupt"'*)
      printf '{"id":%s,"result":{}}\n' "$id"
      printf '{"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"interrupted"}}}\n'
      ;;
    *'"method":"thread/queue/delete"'*)
      printf '{"id":%s,"result":{"deleted":true}}\n' "$id"
      ;;
    *'"method":"turn/start"'*)
      printf '{"id":%s,"result":{"turn":{"id":"turn-2","status":"inProgress"}}}\n' "$id"
      printf '{"method":"item/started","params":{"threadId":"thread-1","turnId":"turn-2","item":{"type":"userMessage","clientId":"farcaster-handoff-5","content":[{"type":"text","text":"steer now\\n\\nthen queue"}]}}}\n'
      ;;
  esac
done
"#;
    let mut session = test_session();
    session.child.wait().expect("reap initial child");
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(SCRIPT)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn Codex protocol fixture");
    let writer = child.stdin.take().expect("fixture stdin");
    let stdout = child.stdout.take().expect("fixture stdout");
    let (sender, incoming) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        loop {
            let message = read_message(&mut reader);
            let failed = message.is_err();
            if sender.send(message).is_err() || failed {
                break;
            }
        }
    });
    session.child = child;
    session.writer = Some(writer);
    session.incoming = incoming;
    session.native_queue = true;
    session.current_turn = Some("turn-1".into());
    session
        .submit_prompt(
            "steer-1".into(),
            "steer now".into(),
            WorkerSendMode::Steer,
            Vec::new(),
        )
        .expect("send steer");
    session
        .submit_prompt(
            "queue-1".into(),
            "then queue".into(),
            WorkerSendMode::Queue,
            Vec::new(),
        )
        .expect("send queue");
    session.apply_steering().expect("first escape");

    let mut delivered = Vec::new();
    for _ in 0..200 {
        if let Some(WorkerEvent::Activity(WorkerActivity::SubmittedInputDelivered {
            submission_id,
            ..
        })) = session.poll()
        {
            delivered.push(submission_id);
            if delivered.len() == 2 {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(delivered, ["steer-1", "queue-1"]);
    assert!(matches!(session.poll_prompt_ack(), Some((id, Ok(()))) if id == "queue-1"));
    assert!(matches!(session.poll_prompt_ack(), Some((id, Ok(()))) if id == "steer-1"));
    assert!(session.handoff.is_none());
}

#[test]
fn handoff_retries_only_turn_races_and_validates_batch_before_admission() {
    use std::io::BufRead as _;

    for (message, retries) in [
        ("no active turn to steer", true),
        ("cannot steer a review turn", false),
    ] {
        let (mut session, mut sent) = writable_test_session();
        session.current_turn = Some("turn-1".into());
        session
            .submit_prompt(
                "steer-1".into(),
                "pending steer".into(),
                WorkerSendMode::Steer,
                Vec::new(),
            )
            .expect("send steer");
        let mut line = String::new();
        sent.read_line(&mut line).expect("steer request");
        session.apply_steering().expect("first escape");
        line.clear();
        sent.read_line(&mut line).expect("interrupt request");
        session.queued_inbound.push_back(Ok(CodexInbound::Error {
            id: CodexRequestId::Number(1),
            error: super::super::contract::CodexRpcError {
                code: -32000,
                message: message.into(),
                data: json!({"codexErrorInfo":{"type":"activeTurnNotSteerable",
                    "turnKind":"review"}}),
            },
        }));
        let _ = session.poll();
        session
            .queued_inbound
            .push_back(Ok(CodexInbound::Notification {
                method: "turn/completed".into(),
                params: json!({"threadId":"thread-1","turn":{
                    "id":"turn-1","status":"interrupted"
                }}),
            }));
        let _ = session.poll();
        if !retries {
            assert!(matches!(session.poll_prompt_ack(), Some((id, Err(_))) if id == "steer-1"));
            assert_eq!(session.next_id, 2, "policy rejection must not be replayed");
            continue;
        }

        line.clear();
        sent.read_line(&mut line).expect("handoff turn start");
        session.queued_inbound.push_back(Ok(CodexInbound::Response {
            id: CodexRequestId::Number(3),
            result: json!({}),
        }));
        assert!(matches!(
            session.poll(),
            Some(WorkerEvent::PromptDeliveryUnknown { submission_id, error })
                if submission_id == "steer-1" && error.contains("missing field")
        ));
        assert_eq!(
            session.poll_prompt_ack(),
            None,
            "malformed batch response is not admission"
        );
    }
}

#[test]
fn interrupted_completion_waits_for_original_steer_ownership() {
    use std::io::BufRead as _;

    for accepted in [true, false] {
        let (mut session, mut sent) = writable_test_session();
        session.current_turn = Some("turn-1".into());
        session
            .submit_prompt(
                "steer-1".into(),
                "pending steer".into(),
                WorkerSendMode::Steer,
                Vec::new(),
            )
            .expect("submit steer");
        session.apply_steering().expect("first escape");
        for _ in 0..2 {
            let mut line = String::new();
            sent.read_line(&mut line).expect("steer and interrupt");
        }
        session
            .queued_inbound
            .push_back(Ok(CodexInbound::Notification {
                method: "turn/completed".into(),
                params: json!({"threadId":"thread-1","turn":{
                    "id":"turn-1","status":"interrupted"
                }}),
            }));
        let _ = session.poll();
        assert_eq!(session.next_id, 2, "unanswered steer is not owned");

        if accepted {
            session.queued_inbound.push_back(Ok(CodexInbound::Response {
                id: CodexRequestId::Number(1),
                result: json!({"turnId":"turn-1"}),
            }));
        } else {
            session.queued_inbound.push_back(Ok(CodexInbound::Error {
                id: CodexRequestId::Number(1),
                error: super::super::contract::CodexRpcError {
                    code: -32000,
                    message: "no active turn to steer".into(),
                    data: Value::Null,
                },
            }));
        }
        let _ = session.poll();
        let mut line = String::new();
        sent.read_line(&mut line).expect("owned replacement batch");
        let batch: Value = serde_json::from_str(&line).expect("decode batch");
        assert_eq!(batch["method"], "turn/start");
        if accepted {
            assert!(matches!(session.poll_prompt_ack(), Some((id, Ok(()))) if id == "steer-1"));
        } else {
            assert_eq!(session.poll_prompt_ack(), None);
            session.queued_inbound.push_back(Ok(CodexInbound::Response {
                id: CodexRequestId::Number(3),
                result: json!({"turn":{"id":"turn-2","status":"inProgress"}}),
            }));
            let _ = session.poll();
            assert!(matches!(session.poll_prompt_ack(), Some((id, Ok(()))) if id == "steer-1"));
        }
        assert_eq!(session.poll_prompt_ack(), None, "admission is emitted once");
    }
}

#[test]
fn committed_original_steer_before_rpc_reply_is_never_replayed() {
    let (mut session, _sent) = writable_test_session();
    session.current_turn = Some("turn-1".into());
    session
        .submit_prompt(
            "steer-1".into(),
            "already committed".into(),
            WorkerSendMode::Steer,
            Vec::new(),
        )
        .expect("submit steer");
    session.apply_steering().expect("first escape");
    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-1","turn":{
                "id":"turn-1","status":"interrupted"
            }}),
        }));
    let _ = session.poll();
    session
        .queued_inbound
        .push_back(Ok(CodexInbound::Notification {
            method: "item/started".into(),
            params: json!({"threadId":"thread-1","turnId":"turn-1","item":{
                "type":"userMessage","clientId":"farcaster-steer-1",
                "content":[{"type":"text","text":"already committed"}]
            }}),
        }));
    assert!(matches!(
        session.poll(),
        Some(WorkerEvent::Activity(WorkerActivity::SubmittedInputDelivered {
            submission_id,
            ..
        })) if submission_id == "steer-1"
    ));
    session.queued_inbound.push_back(Ok(CodexInbound::Response {
        id: CodexRequestId::Number(1),
        result: json!({"turnId":"turn-1"}),
    }));
    assert!(session.poll().is_none());
    assert_eq!(session.next_id, 2, "committed input is not batched again");
    assert!(session.handoff.is_none());
}

#[test]
fn peer_steer_during_codex_stream_does_not_split_visible_assistant_text() {
    use crate::agents::{SessionEvent, SessionTransport, WorkerActivityState};
    use crate::app::views::transcript::conversation::{ConversationState, TranscriptKind};
    use crate::modules::agents::adapter::main_session::{
        MainSessionMetadata, WorkerSessionTransport,
    };

    let (mut session, _sent) = writable_test_session();
    let (incoming, receiver) = mpsc::channel();
    session.incoming = receiver;
    session.current_turn = Some("turn-1".into());
    session.output = "hello ".into();
    session
        .caller_identity
        .set_activity(WorkerActivityState::Working);
    session.events.extend([
        WorkerEvent::Started,
        WorkerEvent::Activity(WorkerActivity::TextDelta {
            content_index: 0,
            delta: "hello ".into(),
        }),
    ]);
    session.peer_messages.push_back(crate::agents::PeerMessage {
        from: "reviewer".into(),
        message: "keep going".into(),
    });
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "codex-cli",
        "thread-1".into(),
        Box::new(session),
        MainSessionMetadata::default(),
        None,
    )
    .expect("Codex transport");
    incoming
        .send(Ok(CodexInbound::Notification {
            method: "item/agentMessage/delta".into(),
            params: json!({"threadId":"thread-1","turnId":"turn-1","delta":"world"}),
        }))
        .expect("continued delta");
    incoming
        .send(Ok(CodexInbound::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-1","turn":{"id":"turn-1","status":"completed"}}),
        }))
        .expect("turn completion");

    let mut conversation = ConversationState::default();
    while let Some(event) = transport.poll() {
        if let SessionEvent::Activity(activity) = event {
            conversation.reduce(activity.value());
        }
    }
    assert_eq!(
        conversation
            .items
            .iter()
            .filter(|item| item.kind == TranscriptKind::Assistant)
            .map(|item| item.complete_text())
            .collect::<Vec<_>>(),
        ["hello world"]
    );
}
