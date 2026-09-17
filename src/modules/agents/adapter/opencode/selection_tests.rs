use super::*;
use crate::agents::Backend;
use crate::agents::{
    SessionCommand, SessionEvent, SessionResponsePayload as Payload, SessionTransport,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

fn request(
    transport: &mut main_session::WorkerSessionTransport,
    command: SessionCommand,
) -> Payload {
    transport.send(command).expect("send fixture command");
    let Some(SessionEvent::Response(response)) = transport.poll() else {
        panic!("expected response")
    };
    response.result.expect("fixture command should succeed")
}

fn state(transport: &mut main_session::WorkerSessionTransport) -> crate::protocol::SessionState {
    let Payload::LoadState(state) = request(transport, SessionCommand::LoadState) else {
        panic!("expected state")
    };
    *state
}

fn context_limit(transport: &mut main_session::WorkerSessionTransport) -> u64 {
    let Payload::LoadUsage(usage) = request(transport, SessionCommand::LoadUsage) else {
        panic!("expected usage")
    };
    usage.context_usage.expect("context usage").context_window
}

fn wait_for_context_window(
    transport: &mut main_session::WorkerSessionTransport,
    expected: u64,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(SessionEvent::Response(response)) = transport.poll()
            && let Ok(Payload::LoadState(state)) = response.result
            && state
                .model
                .as_ref()
                .is_some_and(|model| model.context_window == expected)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "model catalog update did not publish context window {expected}"
            ));
        }
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn resumed_worker_keeps_model_limits_and_effort_in_sync() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let catalog_limit = Arc::new(AtomicU64::new(272_000));
    let served_catalog_limit = catalog_limit.clone();
    let (catalog_refresh, refresh_requested) = mpsc::channel();
    let (event_ready, event_connected) = mpsc::channel();
    let server = thread::spawn(move || -> Result<(), String> {
        let mut selection = json!({"id": "astra", "providerID": "openai", "variant": "high"});
        let mut event_stream: Option<std::net::TcpStream> = None;
        while !stopped.load(Ordering::Relaxed) {
            if let Ok(limit) = refresh_requested.try_recv() {
                served_catalog_limit.store(limit, Ordering::Relaxed);
                writeln!(
                    event_stream.as_mut().expect("event stream"),
                    "data: {}\n",
                    json!({"type": "model.updated", "data": {}})
                )
                .map_err(|error| error.to_string())?;
            }
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(error) => return Err(error.to_string()),
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .map_err(|error| error.to_string())?;
            let mut header = Vec::new();
            let mut byte = [0];
            while !header.ends_with(b"\r\n\r\n") {
                stream
                    .read_exact(&mut byte)
                    .map_err(|error| error.to_string())?;
                header.push(byte[0]);
            }
            let header = String::from_utf8(header).map_err(|error| error.to_string())?;
            let length = header
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            let mut body = vec![0; length];
            stream
                .read_exact(&mut body)
                .map_err(|error| error.to_string())?;
            let body: Value = if body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&body).map_err(|error| error.to_string())?
            };
            let path = header.split_whitespace().nth(1).ok_or("request path")?;
            let (status, response) = if path.starts_with("/api/model?") {
                (
                    200,
                    json!({"data": [
                        {"id": "astra", "name": "Astra", "providerID": "provider", "variants": ["thinking"], "limit": {"context": 1050000}},
                        {"id": "astra", "name": "Astra", "providerID": "openai", "variants": ["high", "low"], "limit": {"context": 400000, "input": served_catalog_limit.load(Ordering::Relaxed)}},
                        {"id": "fixed", "name": "Fixed", "providerID": "openai", "variants": [], "limit": {"context": 64000, "input": 0}}
                    ]}),
                )
            } else if path.starts_with("/api/agent?") || path.starts_with("/api/command?") {
                (200, json!({"data": []}))
            } else if path == "/api/session/ses_resume" {
                (
                    200,
                    json!({"data": {"id": "ses_resume", "location": {"directory": "/project"},
                    "model": selection}}),
                )
            } else if path == "/api/event" {
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").map_err(|error| error.to_string())?;
                event_stream = Some(stream);
                event_ready.send(()).map_err(|error| error.to_string())?;
                continue;
            } else if path == "/api/session/ses_resume/model" {
                recorded
                    .lock()
                    .expect("test lock should not be poisoned")
                    .push(body.clone());
                if body["model"]["variant"] == "high" {
                    (400, json!({"message": "selection rejected"}))
                } else {
                    selection = body["model"].clone();
                    if selection.get("variant").is_none() {
                        selection["variant"] = json!("default");
                    }
                    if selection["id"] == "fixed" {
                        let event = json!({"type": "session.step.ended", "data": {
                            "sessionID": "ses_resume", "tokens": {"input": 12000, "output": 100}
                        }});
                        writeln!(
                            event_stream.as_mut().expect("event stream"),
                            "data: {event}\n"
                        )
                        .map_err(|error| error.to_string())?;
                    }
                    (204, Value::Null)
                }
            } else {
                return Err(format!("unexpected request: {path}"));
            };
            let response = response.to_string();
            write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).map_err(|error| error.to_string())?;
        }
        Ok(())
    });
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let script = project.path().join("opencode-selection.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf '{{\"url\":\"http://{address}\"}}\\n'\ncat\n"),
    )
    .map_err(|error| error.to_string())?;
    let mut command = AgentLaunchConfig::test_script(&script, vec![]);
    command.access_mode = crate::agents::HarnessAccessMode::Sandboxed;
    let result = (|| {
        let launch = crate::agents::SessionLaunch {
            harness: Backend::OpenCode,
            session_id: Some("ses_resume".into()),
            project: project.path().to_owned(),
            start: crate::agents::SessionStart::Resume(project.path().join("session")),
            wake: None,
        };
        let open = || {
            let (worker, locator, metadata) = spawn_main(&command, &launch)?;
            event_connected
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| error.to_string())?;
            assert!(metadata.efforts.is_empty());
            assert_eq!(metadata.models[0]["contextWindow"], 1050000);
            main_session::WorkerSessionTransport::new(
                project.path(),
                Backend::OpenCode,
                locator,
                worker,
                metadata,
                Some(crate::agents::DiscoveredHistory {
                    messages: vec![],
                    model: Some(("provider".into(), "other".into())),
                    thinking_level: Some("thinking".into()),
                    prompt_deliveries: None,
                }),
            )
        };
        let mut transport = open()?;
        let initial = state(&mut transport);
        assert_eq!(initial.thinking_level.as_deref(), Some("high"));
        let model = initial.model.expect("initial model");
        assert_eq!(model.id, "astra");
        assert_eq!(model.name, "Astra");
        assert_eq!(model.context_window, 272000);
        assert_eq!(context_limit(&mut transport), 272000);
        assert_eq!(model.efforts.expect("model variants"), ["high", "low"]);
        catalog_refresh
            .send(200_000)
            .map_err(|error| error.to_string())?;
        wait_for_context_window(&mut transport, 200_000)?;
        assert_eq!(context_limit(&mut transport), 200_000);
        assert_eq!(
            request(&mut transport, SessionCommand::ListReasoningLevels),
            Payload::ListReasoningLevels(vec!["low".into(), "high".into()])
        );
        request(
            &mut transport,
            SessionCommand::SelectReasoning {
                level: "low".into(),
            },
        );
        assert_eq!(state(&mut transport).thinking_level.as_deref(), Some("low"));
        assert!(
            transport
                .send(SessionCommand::SelectReasoning {
                    level: "high".into()
                })
                .is_err()
        );
        assert_eq!(state(&mut transport).thinking_level.as_deref(), Some("low"));
        assert_eq!(context_limit(&mut transport), 200000);
        // A preset advertised only by another model must not be sent.
        assert!(
            transport
                .send(SessionCommand::SelectReasoning {
                    level: "thinking".into()
                })
                .is_err()
        );
        request(&mut transport, SessionCommand::ResetReasoning);
        assert_eq!(state(&mut transport).thinking_level, None);
        transport.close()?;
        let mut transport = open()?;
        assert_eq!(state(&mut transport).thinking_level, None);
        request(
            &mut transport,
            SessionCommand::SelectReasoning {
                level: "low".into(),
            },
        );
        request(
            &mut transport,
            SessionCommand::SelectModel {
                provider: "openai".into(),
                model_id: "fixed".into(),
            },
        );
        assert_eq!(state(&mut transport).thinking_level, None);
        assert_eq!(
            request(&mut transport, SessionCommand::ListReasoningLevels),
            Payload::ListReasoningLevels(vec![])
        );
        assert_eq!(context_limit(&mut transport), 64000);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(SessionEvent::Activity(activity)) = transport.poll()
                && activity.value()["type"] == "turn_end"
            {
                assert_eq!(activity.value()["contextWindow"], 64000);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "missing usage event after model switch"
            );
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(context_limit(&mut transport), 64000);
        transport.close()?;
        Ok::<_, String>(())
    })();
    stop.store(true, Ordering::Relaxed);
    server.join().map_err(|_| "mock server panicked")??;
    result?;
    let requests = requests.lock().expect("test lock should not be poisoned");
    assert_eq!(requests.len(), 5);
    assert_eq!(
        requests[0]["model"],
        json!({"providerID": "openai", "id": "astra", "variant": "low"})
    );
    assert_eq!(
        requests[2]["model"],
        json!({"providerID": "openai", "id": "astra"})
    );
    Ok(())
}
