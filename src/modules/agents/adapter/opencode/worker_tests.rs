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
    let (event_sender, event_receiver) = mpsc::channel::<String>();
    let event_receiver = Arc::new(Mutex::new(event_receiver));
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let directory = project.path().to_string_lossy().into_owned();
    let session_body = serde_json::to_string(&json!({"data": {
        "id": "saved-session", "location": {"directory": directory}
    }}))
    .map_err(|error| error.to_string())?;
    let server = thread::spawn(move || -> Result<(), String> {
        for _ in 0..4 {
            let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
            let recorded = Arc::clone(&recorded);
            let session_body = session_body.clone();
            let event_sender = event_sender.clone();
            let event_receiver = Arc::clone(&event_receiver);
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
                    loop {
                        let event = event_receiver
                            .lock()
                            .map_err(|error| error.to_string())?
                            .recv()
                            .map_err(|error| error.to_string())?;
                        if event == "close" {
                            return Ok(());
                        }
                        write!(stream, "data: {event}\n\n").map_err(|error| error.to_string())?;
                        stream.flush().map_err(|error| error.to_string())?;
                    }
                }
                if request.starts_with("POST /api/session/saved-session/prompt ") {
                    let body = request
                        .split_once("\r\n\r\n")
                        .map(|(_, body)| body)
                        .ok_or("fixture prompt has no body")?;
                    let body: Value =
                        serde_json::from_str(body).map_err(|error| error.to_string())?;
                    let id = body["id"].as_str().ok_or("fixture prompt has no id")?;
                    if body["text"].as_str() == Some("after restart") {
                        event_sender
                            .send(
                                json!({
                                    "id":"direct-delivery", "type":"session.inbox.delivered",
                                    "data":{"sessionID":"saved-session", "inboxID":id}
                                })
                                .to_string(),
                            )
                            .map_err(|error| error.to_string())?;
                        return Ok(());
                    }
                    let response = json!({"data": {
                        "id": id, "sessionID": "saved-session", "delivery": body["delivery"]
                    }})
                    .to_string();
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                        response.len()
                    )
                    .map_err(|error| error.to_string())?;
                    event_sender
                        .send("close".into())
                        .map_err(|error| error.to_string())?;
                    return Ok(());
                }
                let response = if request.starts_with("GET /api/session/saved-session ") {
                    session_body
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
    let unknown_id = match worker.poll() {
        Some(WorkerEvent::PromptDeliveryUnknown { submission_id, .. }) => submission_id,
        event => {
            return Err(format!(
                "expected direct prompt delivery uncertainty, got {event:?}"
            ));
        }
    };
    let uuid = unknown_id
        .strip_prefix("msg_farcaster_")
        .ok_or("direct prompt ID is missing its namespace")?;
    uuid::Uuid::parse_str(uuid).map_err(|error| error.to_string())?;
    let mut delivered = None;
    for _ in 0..500 {
        delivered = worker.poll();
        if delivered.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(matches!(
        delivered,
        Some(WorkerEvent::Activity(WorkerActivity::SubmittedInputDelivered {
            submission_id,
            mode: WorkerSendMode::Prompt,
            message,
        })) if submission_id == unknown_id && message == "after restart"
    ));
    worker.send("still alive".into(), WorkerSendMode::Prompt)?;
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
    assert!(
        requests
            .iter()
            .any(|request| request.contains("still alive"))
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
fn promotion_failures_do_not_skip_later_followups_or_interrupt() {
    use super::super::{
        client::OpenCodeClient,
        contract::{OpenCodeHttpRequest, OpenCodeHttpResponse, OpenCodeHttpTransport},
    };

    struct Transport {
        responses: VecDeque<OpenCodeHttpResponse>,
        requests: Vec<OpenCodeHttpRequest>,
    }
    impl OpenCodeHttpTransport for Transport {
        fn execute(
            &mut self,
            request: OpenCodeHttpRequest,
        ) -> Result<OpenCodeHttpResponse, String> {
            self.requests.push(request);
            self.responses
                .pop_front()
                .ok_or_else(|| "missing response".into())
        }
    }
    let response = |status, body: Value| OpenCodeHttpResponse {
        status,
        body: serde_json::to_vec(&body).expect("test response serializes"),
    };
    let transport = Transport {
        responses: VecDeque::from([
            response(500, json!({"_tag":"Failure", "message":"first failed"})),
            response(204, Value::Null),
            response(200, json!({"interrupted":true})),
        ]),
        requests: Vec::new(),
    };
    let mut client = OpenCodeClient::new(transport);
    let (interrupted, errors) =
        promote_followups_and_interrupt(&mut client, "session-1", ["first", "second"]);
    assert_eq!(interrupted, Some(true));
    assert_eq!(errors.len(), 1);
    let requests = client.into_transport().requests;
    assert!(requests[0].path.ends_with("/inbox/first/steer"));
    assert!(requests[1].path.ends_with("/inbox/second/steer"));
    assert!(requests[2].path.ends_with("/interrupt?continue=true"));
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
            (
                "steer-1".into(),
                PendingOpenCodeDelivery {
                    submission_id: Some("submission-steer".into()),
                    mode: WorkerSendMode::Steer,
                    message: "same text".into(),
                    images: vec![crate::protocol::PromptImage::new(
                        "AQID".into(),
                        "image/png".into(),
                    )],
                    clears_abort_barrier: false,
                },
            ),
            (
                "queue-1".into(),
                PendingOpenCodeDelivery {
                    submission_id: Some("submission-queue".into()),
                    mode: WorkerSendMode::Queue,
                    message: "same text".into(),
                    images: vec![crate::protocol::PromptImage::new(
                        "BAUG".into(),
                        "image/jpeg".into(),
                    )],
                    clears_abort_barrier: false,
                },
            ),
        ]),
        delivered_awaiting_execution: HashSet::new(),
        active_tools: HashMap::new(),
        generation: 0,
        completions: None,
        turn_active: true,
        steering_interrupts: 1,
        ignore_execution_events: false,
        abort_waiting_for_start: false,
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
        Some(WorkerEvent::Activity(WorkerActivity::SubmittedInputDeliveredWithImages {
            submission_id,
            mode: WorkerSendMode::Steer,
            images,
            ..
        })) if submission_id == "submission-steer" && images[0].mime_type == "image/png"
    ));
    assert!(worker.pending_deliveries.contains_key("queue-1"));
    send("session.inbox.delivered", json!({"inboxID": "queue-1"}));
    assert!(matches!(
        worker.poll_native_event(),
        Some(WorkerEvent::Activity(WorkerActivity::SubmittedInputDeliveredWithImages {
            submission_id,
            mode: WorkerSendMode::Queue,
            images,
            ..
        })) if submission_id == "submission-queue" && images[0].mime_type == "image/jpeg"
    ));
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
fn abort_reinterrupts_a_delivery_that_wins_the_cancel_race() -> Result<(), String> {
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
    let fixture = thread::spawn(move || -> Result<(), String> {
        for index in 0..2 {
            let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
            let mut request = Vec::new();
            let mut byte = [0_u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream
                    .read_exact(&mut byte)
                    .map_err(|error| error.to_string())?;
                request.push(byte[0]);
            }
            recorded
                .lock()
                .map_err(|error| error.to_string())?
                .push(String::from_utf8_lossy(&request).into_owned());
            match index {
                0 => {
                    let response = r#"{"interrupted":false}"#;
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{response}",
                        response.len()
                    )
                    .map_err(|error| error.to_string())?;
                }
                _ => {
                    let response = r#"{"interrupted":true}"#;
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{response}",
                        response.len()
                    )
                    .map_err(|error| error.to_string())?;
                }
            }
        }
        Ok(())
    });
    let child = std::process::Command::new("sh")
        .args([
            "-c",
            &format!("printf '{{\"url\":\"http://{address}\"}}\\n'; cat"),
        ])
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
        pending_deliveries: HashMap::from([(
            "msg_delivered".into(),
            PendingOpenCodeDelivery {
                submission_id: Some("delivered".into()),
                mode: WorkerSendMode::Queue,
                message: "delivered".into(),
                images: Vec::new(),
                clears_abort_barrier: false,
            },
        )]),
        delivered_awaiting_execution: HashSet::new(),
        active_tools: HashMap::new(),
        generation: 0,
        completions: None,
        turn_active: true,
        steering_interrupts: 0,
        ignore_execution_events: false,
        abort_waiting_for_start: false,
        wake: None,
        pending: VecDeque::new(),
    };

    sender
        .send(Ok(super::super::contract::OpenCodeEvent {
            id: None,
            event: Some("session.inbox.delivered".into()),
            data: json!({"sessionID":"session-1", "inboxID":"msg_delivered"}),
        }))
        .map_err(|error| error.to_string())?;
    assert!(matches!(
        worker.poll_native_event(),
        Some(WorkerEvent::Activity(WorkerActivity::SubmittedInputDelivered {
            submission_id,
            ..
        })) if submission_id == "delivered"
    ));
    worker.abort()?;
    assert!(worker.abort_waiting_for_start);
    assert!(worker.pending.is_empty());
    assert!(worker.pending_deliveries.is_empty());
    sender
        .send(Ok(super::super::contract::OpenCodeEvent {
            id: None,
            event: Some("session.execution.started".into()),
            data: json!({"sessionID":"session-1"}),
        }))
        .map_err(|error| error.to_string())?;
    assert!(matches!(
        worker.poll_native_event(),
        Some(WorkerEvent::Settled { .. })
    ));
    assert!(!worker.abort_waiting_for_start);
    worker.close()?;
    fixture
        .join()
        .map_err(|_| "abort fixture panicked".to_owned())??;
    let requests = requests.lock().map_err(|error| error.to_string())?;
    assert!(requests[0].contains("interrupt?continue=false"));
    assert!(requests[1].contains("interrupt?continue=false"));
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
        delivered_awaiting_execution: HashSet::new(),
        active_tools: HashMap::new(),
        generation: 0,
        completions: None,
        turn_active: true,
        steering_interrupts: 0,
        ignore_execution_events: false,
        abort_waiting_for_start: false,
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
        Some("queue-1"),
        PendingOpenCodeDelivery {
            submission_id: Some("submission-queue".into()),
            mode: WorkerSendMode::Queue,
            message: "next task".into(),
            images: Vec::new(),
            clears_abort_barrier: false,
        },
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

#[test]
fn http_sse_prompt_and_escape_flow_preserves_exact_delivery_and_liveness() -> Result<(), String> {
    use crate::agents::{SessionCommand, SessionEvent, SessionTransport};
    use crate::app::views::transcript::conversation::{ConversationState, TranscriptKind};
    use crate::modules::agents::adapter::main_session::{
        MainSessionMetadata, WorkerSessionTransport,
    };
    use crate::protocol::PromptMode;
    use std::{
        io::{Read as _, Write as _},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    enum FixtureEvent {
        Data(String),
        Shutdown,
    }
    let (event_sender, event_receiver) = mpsc::channel::<FixtureEvent>();
    let (fixture_shutdown_sender, fixture_shutdown_receiver) = mpsc::channel::<()>();
    let fixture_event_sender = event_sender.clone();
    let event_receiver = Arc::new(Mutex::new(event_receiver));
    let recorded = Arc::clone(&requests);
    let fixture = thread::spawn(move || -> Result<(), String> {
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let mut handlers = Vec::new();
        loop {
            match fixture_shutdown_receiver.try_recv() {
                Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
                Err(mpsc::TryRecvError::Empty) => {}
            }
            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(error) => return Err(error.to_string()),
            };
            let timeout = Some(Duration::from_secs(5));
            stream
                .set_read_timeout(timeout)
                .and_then(|()| stream.set_write_timeout(timeout))
                .map_err(|error| error.to_string())?;
            let recorded = Arc::clone(&recorded);
            let event_receiver = Arc::clone(&event_receiver);
            let fixture_event_sender = fixture_event_sender.clone();
            handlers.push(thread::spawn(move || -> Result<(), String> {
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
                    drop(fixture_event_sender);
                    loop {
                        match event_receiver
                            .lock()
                            .map_err(|error| error.to_string())?
                            .recv_timeout(Duration::from_secs(5))
                        {
                            Ok(FixtureEvent::Data(event)) => {
                                write!(stream, "data: {event}\n\n")
                                    .map_err(|error| error.to_string())?;
                                stream.flush().map_err(|error| error.to_string())?;
                            }
                            Ok(FixtureEvent::Shutdown) => return Ok(()),
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                return Err("timed out waiting for OpenCode fixture event".into());
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => {
                                return Err("OpenCode fixture event sender disconnected".into());
                            }
                        }
                    }
                }
                if request.starts_with("POST /api/session/session-1/prompt ") {
                    let body = request
                        .split_once("\r\n\r\n")
                        .map(|(_, body)| body)
                        .ok_or("fixture request has no body")?;
                    let body: Value = serde_json::from_str(body).map_err(|error| error.to_string())?;
                    let id = body["id"].as_str().ok_or("fixture prompt has no id")?;
                    let delivery = body["delivery"]
                        .as_str()
                        .ok_or("fixture prompt has no delivery")?;
                    if body["text"].as_str() == Some("lost reply") {
                        fixture_event_sender
                            .send(FixtureEvent::Data(
                                json!({
                                    "id":"lost-delivery", "type":"session.inbox.delivered",
                                    "data":{"sessionID":"session-1", "inboxID":id}
                                })
                                .to_string(),
                            ))
                            .map_err(|error| error.to_string())?;
                        return Ok(());
                    }
                    let response = json!({"data": {
                        "id": id, "sessionID": "session-1", "delivery": delivery
                    }})
                    .to_string();
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len())
                        .map_err(|error| error.to_string())?;
                    return Ok(());
                }
                if request.starts_with("POST /api/session/session-1/interrupt?") {
                    let response = r#"{"interrupted":true}"#;
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len())
                        .map_err(|error| error.to_string())?;
                    return Ok(());
                }
                if request.starts_with("POST /api/session/session-1/inbox/")
                    && request.contains("/steer ")
                {
                    let inbox_id = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_ascii_whitespace().nth(1))
                        .and_then(|path| path.split('/').nth(5))
                        .ok_or("fixture steer request has no inbox id")?;
                    stream
                        .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .map_err(|error| error.to_string())?;
                    fixture_event_sender
                        .send(FixtureEvent::Data(
                            json!({
                                "id":"promoted-delivery", "type":"session.inbox.delivered",
                                "data":{"sessionID":"session-1", "inboxID":inbox_id}
                            })
                            .to_string(),
                        ))
                        .map_err(|error| error.to_string())?;
                    return Ok(());
                }
                if request.starts_with("DELETE /api/session/session-1/inbox/") {
                    stream
                        .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .map_err(|error| error.to_string())?;
                    return Ok(());
                }
                Err(format!("unexpected fixture request: {request}"))
            }));
        }
        for handler in handlers {
            handler
                .join()
                .map_err(|_| "OpenCode fixture handler panicked".to_owned())??;
        }
        Ok(())
    });

    let child = std::process::Command::new("sh")
        .args([
            "-c",
            &format!("printf '{{\"url\":\"http://{address}\"}}\\n'; cat"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let server = OpenCodeServerProcess::attach(child, "opencode", "test-password")?;
    let incoming = start_event_reader(&server, "session-1", None)?;
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
    let worker = OpenCodeWorkerSession {
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
        delivered_awaiting_execution: HashSet::new(),
        active_tools: HashMap::new(),
        generation: 0,
        completions: None,
        turn_active: false,
        steering_interrupts: 0,
        ignore_execution_events: false,
        abort_waiting_for_start: false,
        wake: None,
        pending: VecDeque::new(),
    };
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "opencode2",
        "session-1".into(),
        Box::new(worker),
        MainSessionMetadata::default(),
        None,
    )?;
    let image =
        |data: &str, mime: &str| crate::protocol::PromptImage::new(data.into(), mime.into());
    let send_prompt = |transport: &mut WorkerSessionTransport,
                       mode,
                       data: &str,
                       mime: &str|
     -> Result<String, String> {
        transport.send(SessionCommand::Prompt {
            mode,
            message: "same text".into(),
            images: vec![image(data, mime)],
        })
    };
    let normal = send_prompt(&mut transport, PromptMode::Normal, "AQID", "image/png")?;
    let steer = send_prompt(&mut transport, PromptMode::Steer, "BAUG", "image/jpeg")?;
    let queue = send_prompt(&mut transport, PromptMode::FollowUp, "BwgJ", "image/webp")?;
    transport.send(SessionCommand::ApplySteering)?;
    let cancelled = send_prompt(&mut transport, PromptMode::FollowUp, "CgsM", "image/gif")?;

    let native = |id: &str| format!("msg_{id}");
    let event = |kind: &str, data: Value| {
        json!({"id":"fixture-event", "type":kind, "data":data}).to_string()
    };
    event_sender
        .send(FixtureEvent::Data(event(
            "session.text.delta",
            json!({"sessionID":"session-1", "delta":"before "}),
        )))
        .map_err(|error| error.to_string())?;
    event_sender
        .send(FixtureEvent::Data(event(
            "session.execution.interrupted",
            json!({"sessionID":"session-1"}),
        )))
        .map_err(|error| error.to_string())?;
    event_sender
        .send(FixtureEvent::Data(event(
            "session.execution.started",
            json!({"sessionID":"session-1"}),
        )))
        .map_err(|error| error.to_string())?;
    for id in [&normal, &steer] {
        event_sender
            .send(FixtureEvent::Data(event(
                "session.inbox.delivered",
                json!({"sessionID":"session-1", "inboxID":native(id)}),
            )))
            .map_err(|error| error.to_string())?;
    }
    event_sender
        .send(FixtureEvent::Data(event(
            "session.text.delta",
            json!({"sessionID":"session-1", "delta":"after"}),
        )))
        .map_err(|error| error.to_string())?;

    let mut conversation = ConversationState::default();
    let mut receipt_statuses = HashMap::<String, Vec<String>>::new();
    for _ in 0..500 {
        let Some(event) = transport.poll() else {
            thread::sleep(Duration::from_millis(1));
            continue;
        };
        if let SessionEvent::Activity(activity) = event {
            let value = activity.value();
            if value["type"].as_str() == Some("prompt_delivery")
                && let (Some(id), Some(status)) =
                    (value["submissionId"].as_str(), value["status"].as_str())
            {
                receipt_statuses
                    .entry(id.to_owned())
                    .or_default()
                    .push(status.to_owned());
            }
            conversation.reduce(value);
        }
        let assistant = conversation
            .items
            .iter()
            .filter(|item| item.kind == TranscriptKind::Assistant)
            .map(|item| item.complete_text())
            .collect::<String>();
        if assistant == "before after" {
            break;
        }
    }
    assert_eq!(
        conversation
            .items
            .iter()
            .filter(|item| item.kind == TranscriptKind::Assistant)
            .map(|item| item.complete_text())
            .collect::<String>(),
        "before after"
    );
    let retained = conversation
        .items
        .iter()
        .filter(|item| item.kind == TranscriptKind::User && !item.images.is_empty())
        .count();
    assert_eq!(retained, 4);
    let ids_with_status = |status: &str| {
        receipt_statuses
            .iter()
            .filter(|(_, statuses)| statuses.iter().any(|candidate| candidate == status))
            .map(|(id, _)| id.clone())
            .collect::<HashSet<_>>()
    };
    assert_eq!(
        ids_with_status("accepted"),
        HashSet::from([
            normal.clone(),
            steer.clone(),
            queue.clone(),
            cancelled.clone(),
        ])
    );
    assert_eq!(
        ids_with_status("delivered"),
        HashSet::from([normal.clone(), steer.clone(), queue.clone()]),
        "cancelled follow-up must remain accepted but not delivered"
    );

    transport.send(SessionCommand::Abort)?;
    event_sender
        .send(FixtureEvent::Data(event(
            "session.text.delta",
            json!({"sessionID":"session-1", "delta":" stale"}),
        )))
        .map_err(|error| error.to_string())?;
    event_sender
        .send(FixtureEvent::Data(event(
            "session.execution.started",
            json!({"sessionID":"session-1"}),
        )))
        .map_err(|error| error.to_string())?;
    let live = send_prompt(&mut transport, PromptMode::Normal, "DQ4P", "image/avif")?;
    event_sender
        .send(FixtureEvent::Data(event(
            "session.inbox.delivered",
            json!({"sessionID":"session-1", "inboxID":native(&live)}),
        )))
        .map_err(|error| error.to_string())?;
    event_sender
        .send(FixtureEvent::Data(event(
            "session.text.delta",
            json!({"sessionID":"session-1", "delta":"alive"}),
        )))
        .map_err(|error| error.to_string())?;
    for _ in 0..500 {
        let Some(event) = transport.poll() else {
            thread::sleep(Duration::from_millis(1));
            continue;
        };
        if let SessionEvent::Activity(activity) = event {
            conversation.reduce(activity.value());
        }
    }
    let assistant = conversation
        .items
        .iter()
        .filter(|item| item.kind == TranscriptKind::Assistant)
        .map(|item| item.complete_text())
        .collect::<String>();
    assert!(!assistant.contains("stale"));
    assert!(assistant.contains("alive"));

    let lost = transport.send(SessionCommand::Prompt {
        mode: PromptMode::FollowUp,
        message: "lost reply".into(),
        images: vec![image("EBES", "image/png")],
    })?;
    for _ in 0..500 {
        let Some(event) = transport.poll() else {
            thread::sleep(Duration::from_millis(1));
            continue;
        };
        if let SessionEvent::Activity(activity) = event {
            conversation.reduce(activity.value());
        }
    }
    assert_eq!(
        conversation
            .items
            .iter()
            .filter(|item| item.kind == TranscriptKind::User && item.text == "lost reply")
            .count(),
        1
    );
    let history = super::super::catalog::history_messages(&json!({
        "id": native(&lost),
        "type": "user",
        "text": "lost reply",
        "files": [{"mime":"image/png", "data":"EBES"}],
    }));
    conversation.replace_history(&history);
    assert_eq!(
        conversation
            .items
            .iter()
            .filter(|item| item.kind == TranscriptKind::User && item.text == "lost reply")
            .count(),
        1
    );
    assert_eq!(
        conversation
            .items
            .iter()
            .find(|item| item.kind == TranscriptKind::User && item.text == "lost reply")
            .map(|item| item.images.len()),
        Some(1)
    );

    event_sender
        .send(FixtureEvent::Shutdown)
        .map_err(|error| error.to_string())?;
    fixture_shutdown_sender
        .send(())
        .map_err(|error| error.to_string())?;
    transport.close()?;
    fixture
        .join()
        .map_err(|_| "OpenCode fixture panicked".to_owned())??;
    let requests = requests.lock().map_err(|error| error.to_string())?;
    let relevant = requests
        .iter()
        .filter_map(|request| request.lines().next())
        .filter(|request| !request.starts_with("GET /api/event "))
        .collect::<Vec<_>>();
    assert!(relevant[3].starts_with(&format!(
        "POST /api/session/session-1/inbox/{}/steer ",
        native(&queue)
    )));
    assert!(relevant[4].contains("continue=true"));
    assert!(relevant[6].contains("continue=false"));
    assert!(relevant[7].starts_with(&format!(
        "DELETE /api/session/session-1/inbox/{} ",
        native(&cancelled)
    )));
    assert!(relevant[8].starts_with("POST /api/session/session-1/prompt "));
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.contains("\"text\":\"lost reply\""))
            .count(),
        1,
        "an unknown admission must never be replayed"
    );
    assert!(lost.starts_with("opencode2-"));
    Ok(())
}
