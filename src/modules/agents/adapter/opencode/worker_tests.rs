use super::*;
use serde_json::json;

#[test]
fn worker_factory_resumes_the_saved_session_and_accepts_a_new_prompt() -> Result<(), String> {
    use std::{
        io::{Read as _, Write as _},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let directory = project.path().to_string_lossy().into_owned();
    let session_body = serde_json::to_string(&json!({"data": {
        "id": "saved-session", "location": {"directory": directory}
    }}))
    .map_err(|error| error.to_string())?;
    let server = thread::spawn(move || -> Result<(), String> {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
            let recorded = Arc::clone(&recorded);
            let session_body = session_body.clone();
            thread::spawn(move || -> Result<(), String> {
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    stream
                        .read_exact(&mut byte)
                        .map_err(|error| error.to_string())?;
                    request.push(byte[0]);
                }
                let headers = String::from_utf8_lossy(&request);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':').and_then(|(name, value)| {
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                    })
                    .unwrap_or(0);
                let mut body = vec![0; length];
                stream
                    .read_exact(&mut body)
                    .map_err(|error| error.to_string())?;
                request.extend(body);
                let request = String::from_utf8_lossy(&request).into_owned();
                recorded
                    .lock()
                    .map_err(|error| error.to_string())?
                    .push(request.clone());
                if request.starts_with("GET /api/event ") {
                    stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n")
                        .map_err(|error| error.to_string())?;
                    return Ok(());
                }
                let response = if request.starts_with("GET /api/session/saved-session ") {
                    session_body
                } else if request.starts_with("POST /api/session/saved-session/prompt ") {
                    r#"{"data":{"id":"prompt-1","sessionID":"saved-session","delivery":"queue"}}"#
                        .into()
                } else {
                    return Err(format!("unexpected request: {request}"));
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len()
                )
                .map_err(|error| error.to_string())
            });
        }
        Ok(())
    });
    let script = project.path().join("opencode-resume-fixture.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf '{{\"url\":\"http://{address}\"}}\\n'\ncat\n"),
    )
    .map_err(|error| error.to_string())?;
    let factory = OpenCodeWorkerFactory::new(AgentLaunchConfig::test_script(&script, Vec::new()));
    let mut worker = factory.create(WorkerLaunch {
        slot: None,
        worker_id: "resumed-worker".into(),
        worker_name: "resumed".into(),
        project: project.path().to_owned(),
        parent_session: "parent-session".into(),
        parent_worker_id: None,
        context: WorkerContext::Resume {
            session_locator: "saved-session".into(),
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
            locator: "saved-session".into(),
        })
    );
    worker.send("after restart".into(), WorkerSendMode::Prompt)?;
    worker.close()?;
    server
        .join()
        .map_err(|_| "fixture server panicked".to_owned())??;

    let requests = requests.lock().map_err(|error| error.to_string())?;
    assert!(
        requests
            .iter()
            .any(|request| request.starts_with("GET /api/session/saved-session "))
    );
    assert!(!requests.iter().any(|request| request.contains("/fork")));
    assert!(
        requests
            .iter()
            .any(|request| request.contains("after restart"))
    );
    Ok(())
}

#[test]
fn child_execution_events_publish_sidebar_metadata() {
    for (kind, parent, running) in [
        ("session.execution.started", "parent-1", Some(true)),
        ("session.execution.started.1", "parent-1", Some(true)),
        ("session.execution.succeeded", "parent-1", Some(false)),
        ("session.execution.failed", "parent-1", Some(false)),
        ("session.execution.interrupted", "parent-1", Some(false)),
        ("session.execution.started", "unrelated", None),
    ] {
        let event = super::super::contract::OpenCodeEvent {
            id: None,
            event: Some(kind.into()),
            data: json!({"sessionID": "child-1"}),
        };
        let activity = opencode_child_activity(&event, "parent-1", |id| {
            assert_eq!(id, "child-1");
            serde_json::from_value(json!({
                "id": id, "parentID": parent,
                "location": {"directory": "/project"},
                "title": "Explore code",
            }))
            .map_err(|error| error.to_string())
        })
        .expect("session lookup");
        let actual = activity.map(|activity| {
            let WorkerActivity::ChildSessionsChanged {
                id,
                title,
                is_running,
                outcome: _,
            } = activity
            else {
                panic!("expected child metadata");
            };
            assert_eq!(id, "child-1");
            assert_eq!(title.as_deref(), Some("Explore code"));
            is_running
        });
        assert_eq!(actual, running, "{kind} with parent {parent}");
    }
}

#[test]
fn child_observation_skips_parent_text_and_malformed_events() {
    for (kind, data) in [
        (
            "session.execution.started",
            json!({"sessionID": "parent-1"}),
        ),
        (
            "session.text.delta",
            json!({"sessionID": "child-1", "delta": "text"}),
        ),
        ("session.execution.started", json!({})),
        ("session.execution.started", json!({"sessionID": ""})),
    ] {
        let event = super::super::contract::OpenCodeEvent {
            id: None,
            event: Some(kind.into()),
            data,
        };
        assert!(
            opencode_child_activity(&event, "parent-1", |_| {
                panic!("unrelated events must not query the server")
            })
            .expect("test operation should succeed")
            .is_none()
        );
    }
}

#[test]
fn cli_model_fallback_preserves_provider_and_nested_model_ids() {
    assert_eq!(
        models_from_cli("openai/gpt-5\nopenrouter/anthropic/claude\nnoise\n"),
        vec![
            json!({"id":"gpt-5","name":"gpt-5","provider":"openai","contextWindow":0,"reasoning":true}),
            json!({"id":"anthropic/claude","name":"anthropic/claude","provider":"openrouter","contextWindow":0,"reasoning":true}),
        ]
    );
}

#[test]
fn session_updates_surface_titles() {
    // `session.renamed` carries a flat title; this is what title generation
    // and renames emit on the installed opencode2 server.
    let renamed = json!({
        "sessionID": "session-1",
        "title": "Renamed probe title"
    });
    assert_eq!(
        opencode_session_title(&renamed).as_deref(),
        Some("Renamed probe title")
    );

    // `session.updated` nests the full session record under `info`.
    let updated = json!({
        "sessionID": "session-1",
        "info": {"id": "session-1", "title": "Refactor adapter"}
    });
    assert_eq!(
        opencode_session_title(&updated).as_deref(),
        Some("Refactor adapter")
    );

    for data in [
        json!({}),
        json!({"title": ""}),
        json!({"info": {}}),
        json!({"info": {"title": ""}}),
    ] {
        assert!(opencode_session_title(&data).is_none(), "{data}");
    }
}

#[test]
fn current_opencode_events_and_tool_results_are_normalized() {
    assert_eq!(
        unversioned_opencode_event_type("session.next.step.ended.2"),
        "session.next.step.ended"
    );
    assert_eq!(
        opencode_tool_result(&json!({"result": {"answer": 42}}), false),
        json!([{"type":"text", "text":"{\"answer\":42}"}])
    );
    assert_eq!(
        opencode_tool_result(&json!({"error": {"message": "denied"}}), true),
        json!([{"type":"text", "text":"denied"}])
    );
}

#[test]
fn tool_native_updates_merge_without_losing_input_or_title() {
    let mut native = json!({
        "name": "read_file",
        "input": {"filePath": "src/main.rs"},
        "metadata": {"title": "Inspect source", "phase": "starting"}
    });
    merge_opencode_native(
        &mut native,
        &json!({"metadata": {"phase": "running", "percent": 50}}),
    );
    assert_eq!(native["name"], "read_file");
    assert_eq!(native["input"]["filePath"], "src/main.rs");
    assert_eq!(native["metadata"]["title"], "Inspect source");
    assert_eq!(native["metadata"]["phase"], "running");
    assert_eq!(native["metadata"]["percent"], 50);
}

#[test]
fn opencode_model_efforts_accept_current_and_legacy_shapes() {
    assert_eq!(
        model_variant_efforts(&json!({
            "variants": ["low", {"id": "high"}]
        })),
        ["low", "high"]
    );
}

#[test]
fn an_effort_unknown_to_the_target_model_is_never_sent_as_a_variant() {
    let efforts = vec!["low".to_owned(), "medium".to_owned()];
    assert_eq!(variant_for_model(Some("off"), Some(&efforts)), None);
    assert_eq!(variant_for_model(Some("off"), None), Some("off".into()));
    assert_eq!(variant_for_model(None, Some(&efforts)), None);
    assert_eq!(variant_for_model(Some("high"), Some(&efforts)), None);
    assert_eq!(
        variant_for_model(Some("low"), Some(&efforts)),
        Some("low".into())
    );
    assert_eq!(variant_for_model(Some("low"), None), Some("low".into()));
}

#[test]
fn effort_catalog_preserves_known_empty_models_and_skips_unknown_ones() {
    let metadata = crate::modules::agents::adapter::main_session::MainSessionMetadata {
        models: vec![
            json!({
                "id": "kimi-k2.6",
                "provider": "opencode",
                "efforts": [],
            }),
            json!({
                "id": "gpt-5.4",
                "provider": "opencode",
                "efforts": ["none", "low", "high"],
            }),
            json!({
                "id": "legacy",
                "provider": "opencode",
            }),
        ],
        ..Default::default()
    };

    let catalog = effort_catalog(&metadata);
    assert_eq!(
        catalog.get(&("opencode".to_owned(), "kimi-k2.6".to_owned())),
        Some(&Vec::<String>::new())
    );
    assert_eq!(
        catalog.get(&("opencode".to_owned(), "gpt-5.4".to_owned())),
        Some(&vec![
            "none".to_owned(),
            "low".to_owned(),
            "high".to_owned()
        ])
    );
    assert!(!catalog.contains_key(&("opencode".to_owned(), "legacy".to_owned())));
}

#[test]
fn session_usage_totals_are_adopted_without_inflating_the_context_metric() {
    let mut tracker = OpenCodeUsageTracker::default();
    let tokens = |input: u64, output: u64, read: u64| TokenUsage {
        input,
        output,
        cache_read: read,
        cache_write: 0,
    };

    let (turn, session) = tracker.step_ended(tokens(3837, 3, 0));
    assert_eq!((turn.total(), session.total()), (3840, 3840));
    let (turn, session) = tracker.session_total(tokens(4356, 14, 0));
    assert_eq!((turn.total(), session.total()), (3840, 4370));

    let (turn, session) = tracker.step_ended(tokens(72, 4, 3776));
    assert_eq!((turn.total(), session.total()), (3852, 8222));
    let (turn, session) = tracker.session_total(tokens(4428, 18, 3776));
    assert_eq!((turn.input, turn.cache_read, turn.output), (72, 3776, 4));
    assert_eq!(
        (session.input, session.cache_read, session.output),
        (4428, 3776, 18)
    );
}

#[test]
fn permission_requests_keep_child_session_identity() {
    let event = super::super::contract::OpenCodeEvent {
        id: None,
        event: Some("permission.asked".into()),
        data: json!({"sessionID": "child-1", "id": "permission-1"}),
    };

    assert_eq!(
        opencode_permission_request(&event),
        Some(("child-1", "permission-1"))
    );
}

#[test]
fn supported_modes_use_the_opencode_server_without_auto_approval() {
    for mode in [
        crate::agents::HarnessAccessMode::Sandboxed,
        crate::agents::HarnessAccessMode::Full,
    ] {
        let mut command = std::process::Command::new("opencode2");
        configure_opencode_server(&mut command, mode).expect("supported OpenCode mode");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["serve", "--stdio", "--print-logs"]
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == "OPENCODE_DISABLE_AUTOUPDATE")
                .and_then(|(_, value)| value),
            Some(std::ffi::OsStr::new("true"))
        );
    }

    let mut command = std::process::Command::new("opencode2");
    assert_eq!(
        configure_opencode_server(&mut command, crate::agents::HarnessAccessMode::Auto),
        Err("OpenCode does not support model-reviewed automatic approvals".into())
    );
    assert_eq!(command.get_args().count(), 0);
}

#[test]
fn sandboxed_permission_requests_keep_native_choices() {
    let data = json!({
        "permission": "bash",
        "patterns": ["git status", "git diff"]
    });
    assert_eq!(
        opencode_permission_prompt(&data),
        "OpenCode requests permission for bash\ngit status\ngit diff"
    );
    assert_eq!(
        opencode_permission_reply(Some("Allow once"), false),
        Ok("once")
    );
    assert_eq!(
        opencode_permission_reply(Some("Always allow"), false),
        Ok("always")
    );
    assert_eq!(
        opencode_permission_reply(Some("Decline"), false),
        Ok("reject")
    );
    assert_eq!(opencode_permission_reply(None, true), Ok("reject"));
}

#[test]
fn native_startup_merges_direct_farcaster_mcp() {
    let mut command = std::process::Command::new("opencode2");
    command.env(
            "OPENCODE_CONFIG_CONTENT",
            r#"{"model":"provider/model","mcp":{"servers":{"other":{"type":"remote","url":"https://example.test/mcp"}}}}"#,
        );
    configure_farcaster_mcp(&mut command, "caller-1").expect("MCP config");
    let value = command
        .get_envs()
        .find(|(name, _)| *name == "OPENCODE_CONFIG_CONTENT")
        .and_then(|(_, value)| value)
        .and_then(|value| serde_json::from_str::<Value>(&value.to_string_lossy()).ok())
        .expect("inline config");
    assert_eq!(value["model"], "provider/model");
    assert_eq!(
        value["mcp"]["servers"]["other"]["url"],
        "https://example.test/mcp"
    );
    assert_eq!(
        value["mcp"]["servers"]["farcaster"]["url"],
        farcaster_mcp::URL
    );
    assert_eq!(
        value["mcp"]["servers"]["farcaster"]["headers"][farcaster_mcp::CALLER_HEADER],
        "caller-1"
    );
    assert_eq!(value["mcp"]["servers"]["farcaster"]["codemode"], false);
    assert_eq!(value["mcp"]["servers"]["farcaster"]["oauth"], false);
}

#[test]
fn extracts_the_last_assistant_text() {
    let context = [
        json!({"type":"assistant","content":[{"type":"text","text":"old"}]}),
        json!({"type":"user","text":"next"}),
        json!({"type":"assistant","content":[{"type":"reasoning","text":"hidden"},{"type":"text","text":"done"}]}),
    ];
    assert_eq!(final_assistant_text(&context), "done");
}

#[test]
fn steering_interruption_preserves_delivery_and_later_abort_settles() -> Result<(), String> {
    let child = std::process::Command::new("sh")
        .args(["-c", "printf '{\"url\":\"http://127.0.0.1:4096\"}\\n'; cat"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let server = OpenCodeServerProcess::attach(child, "opencode", "test-password")?;
    let (sender, incoming) = mpsc::channel();
    let caller_identity = crate::agents::CallerRegistry::shared().issue(
        std::path::Path::new("/project"),
        crate::modules::agents::core::CallerProfile {
            backend: "opencode2".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    let mut worker = OpenCodeWorkerSession {
        caller_identity,
        server,
        session_id: "session-1".into(),
        provider: None,
        model: None,
        effort: None,
        effort_catalog: HashMap::new(),
        access_mode: crate::agents::HarnessAccessMode::Sandboxed,
        incoming,
        reasoning_started: true,
        text_streams: HashMap::new(),
        reasoning_streams: HashMap::new(),
        usage: OpenCodeUsageTracker::default(),
        context_window: 0,
        pending_inputs: HashMap::new(),
        pending_deliveries: HashMap::from([
            ("steer-1".into(), (WorkerSendMode::Steer, "redirect".into())),
            ("queue-1".into(), (WorkerSendMode::Queue, "later".into())),
        ]),
        active_tools: HashMap::new(),
        generation: 0,
        completions: None,
        turn_active: true,
        steering_interrupts: 1,
        wake: None,
        pending: VecDeque::new(),
    };
    let send = |kind: &str, extra: Value| {
        let mut data = json!({"sessionID": "session-1"});
        data.as_object_mut()
            .expect("test operation should succeed")
            .extend(
                extra
                    .as_object()
                    .expect("test operation should succeed")
                    .clone(),
            );
        sender
            .send(Ok(super::super::contract::OpenCodeEvent {
                id: None,
                event: Some(kind.into()),
                data,
            }))
            .expect("test operation should succeed");
    };
    send("session.execution.interrupted", json!({}));
    send("session.execution.started", json!({}));
    assert!(worker.poll_native_event().is_none());
    assert!(worker.turn_active);
    assert_eq!(worker.pending_deliveries.len(), 2);
    send("session.inbox.delivered", json!({"inboxID": "steer-1"}));
    assert!(matches!(
        worker.poll_native_event(),
        Some(WorkerEvent::Activity(WorkerActivity::InputDelivered {
            mode: WorkerSendMode::Steer,
            ..
        }))
    ));
    assert!(worker.pending_deliveries.contains_key("queue-1"));
    send("session.execution.interrupted", json!({}));
    assert!(matches!(
        worker.poll_native_event(),
        Some(WorkerEvent::Settled { .. })
    ));
    assert!(!worker.turn_active);
    worker.close()?;
    Ok(())
}

#[test]
fn queued_prompt_during_stream_does_not_restart_visible_assistant_text() -> Result<(), String> {
    use crate::agents::{SessionEvent, SessionTransport};
    use crate::app::views::transcript::conversation::{ConversationState, TranscriptKind};
    use crate::modules::agents::adapter::main_session::{
        MainSessionMetadata, WorkerSessionTransport,
    };
    let child = std::process::Command::new("sh")
        .args(["-c", "printf '{\"url\":\"http://127.0.0.1:4096\"}\\n'; cat"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let server = OpenCodeServerProcess::attach(child, "opencode", "test-password")?;
    let (sender, incoming) = mpsc::channel();
    let caller_identity = crate::agents::CallerRegistry::shared().issue(
        std::path::Path::new("/project"),
        crate::modules::agents::core::CallerProfile {
            backend: "opencode2".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    let mut worker = OpenCodeWorkerSession {
        caller_identity,
        server,
        session_id: "session-1".into(),
        provider: None,
        model: None,
        effort: None,
        effort_catalog: HashMap::new(),
        access_mode: crate::agents::HarnessAccessMode::Sandboxed,
        incoming,
        reasoning_started: false,
        text_streams: HashMap::new(),
        reasoning_streams: HashMap::new(),
        usage: OpenCodeUsageTracker::default(),
        context_window: 0,
        pending_inputs: HashMap::new(),
        pending_deliveries: HashMap::new(),
        active_tools: HashMap::new(),
        generation: 0,
        completions: None,
        turn_active: true,
        steering_interrupts: 0,
        wake: None,
        pending: VecDeque::from([
            WorkerEvent::Started,
            WorkerEvent::Activity(WorkerActivity::TextDelta {
                content_index: 0,
                delta: "hello ".into(),
            }),
        ]),
    };
    worker.record_prompt_admission(
        super::super::contract::OpenCodePromptAdmission {
            id: "queue-1".into(),
            session_id: "session-1".into(),
            delivery: "queue".into(),
        },
        WorkerSendMode::Queue,
        "next task".into(),
    )?;
    assert_eq!(
        worker
            .pending
            .iter()
            .filter(|event| matches!(event, WorkerEvent::Started))
            .count(),
        1,
        "queue admission must not emit another turn start"
    );
    sender
        .send(Ok(super::super::contract::OpenCodeEvent {
            id: None,
            event: Some("session.text.delta".into()),
            data: json!({"sessionID":"session-1","delta":"world"}),
        }))
        .map_err(|error| error.to_string())?;
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "opencode2",
        "session-1".into(),
        Box::new(worker),
        MainSessionMetadata::default(),
        None,
    )?;
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
    transport.close()?;
    Ok(())
}
