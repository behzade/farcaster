use super::*;

#[test]
fn native_child_events_refresh_catalog_and_emit_one_finished_activity() {
    let mut session = test_session();
    session.thread_id = "native-parent".into();
    for kind in ["started", "interacted", "interrupted", "completed"] {
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
        assert_eq!(
            session.poll(),
            Some(WorkerEvent::Activity(WorkerActivity::ChildSessionsChanged))
        );
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
            Some(matches!(kind, "started" | "interacted"))
        );
    }
    session.close().unwrap();
    assert_eq!(
        super::super::subagents::is_running("native-event-child"),
        None
    );
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
        native_queue: false,
        next_id: 0,
        current_turn: None,
        output: String::new(),
        reasoning_started: false,
        compacting: false,
        manual_compaction: false,
        pending: HashMap::new(),
        pending_inputs: HashMap::new(),
        queued_inbound: VecDeque::new(),
        peer_messages: VecDeque::new(),
        events: VecDeque::new(),
        turn_error: None,
    }
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
