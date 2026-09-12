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
        prompt_acks: VecDeque::new(),
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
        session.queued_inbound.push_back(Ok(CodexInbound::Response {
            id,
            result: json!({"turn":{"id":"active","status":"inProgress","items":[]}}),
        }));
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
fn rejected_or_malformed_codex_reply_never_acknowledges_success() {
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
        let mut reply = reply;
        reply["id"] = json!(session.next_id);
        session
            .queued_inbound
            .push_back(super::super::wire::decode_frame(
                reply.to_string().as_bytes(),
            ));
        for _ in 0..5 {
            session.poll();
        }
        assert!(matches!(session.poll_prompt_ack(), Some((id, Err(_))) if id == "submission"));
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
            result: json!({}),
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
