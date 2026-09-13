use super::*;
use crate::agents::{
    SessionCommand, SessionEvent, SessionResponsePayload as Payload, SessionTransport,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

fn request(
    transport: &mut main_session::WorkerSessionTransport,
    command: SessionCommand,
) -> Payload {
    transport.send(command).unwrap();
    let Some(SessionEvent::Response(response)) = transport.poll() else {
        panic!("expected response")
    };
    response.result.unwrap()
}

fn state(transport: &mut main_session::WorkerSessionTransport) -> crate::protocol::SessionState {
    let Payload::LoadState(state) = request(transport, SessionCommand::LoadState) else {
        panic!("expected state")
    };
    *state
}
#[test]
fn resumed_worker_changes_and_resets_effort_without_a_model_selection() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let server = thread::spawn(move || -> Result<(), String> {
        let mut selection = json!({"id": "astra", "providerID": "openai", "variant": "high"});
        while !stopped.load(Ordering::Relaxed) {
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
                        {"id": "other", "name": "Other", "providerID": "provider", "variants": ["thinking"]},
                        {"id": "astra", "name": "Astra", "providerID": "openai", "variants": ["high", "low"], "limit": {"context": 400000}},
                        {"id": "fixed", "name": "Fixed", "providerID": "openai", "variants": []}
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
                continue;
            } else if path == "/api/session/ses_resume/model" {
                recorded.lock().unwrap().push(body.clone());
                if body["model"]["variant"] == "high" {
                    (400, json!({"message": "selection rejected"}))
                } else {
                    selection = body["model"].clone();
                    if selection.get("variant").is_none() {
                        selection["variant"] = json!("default");
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
            harness: "opencode".into(),
            session_id: Some("ses_resume".into()),
            project: project.path().to_owned(),
            start: crate::agents::SessionStart::Resume(project.path().join("session")),
            wake: None,
        };
        let open = || {
            let (worker, locator, metadata) = spawn_main(&command, &launch)?;
            assert!(metadata.efforts.is_empty());
            main_session::WorkerSessionTransport::new(
                project.path(),
                "opencode",
                locator,
                worker,
                metadata,
                Some(crate::agents::DiscoveredHistory {
                    messages: vec![],
                    model: Some(("provider".into(), "other".into())),
                    thinking_level: Some("thinking".into()),
                }),
            )
        };
        let mut transport = open()?;
        let initial = state(&mut transport);
        assert_eq!(initial.thinking_level.as_deref(), Some("high"));
        let model = initial.model.unwrap();
        assert_eq!(model.id, "astra");
        assert_eq!(model.name, "Astra");
        assert_eq!(model.context_window, 400000);
        assert_eq!(model.efforts.unwrap(), ["high", "low"]);
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
        transport.close()?;
        Ok::<_, String>(())
    })();
    stop.store(true, Ordering::Relaxed);
    server.join().map_err(|_| "mock server panicked")??;
    result?;
    let requests = requests.lock().unwrap();
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
