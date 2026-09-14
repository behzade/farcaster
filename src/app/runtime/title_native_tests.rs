//! Native title metadata travels through real HTTP/SSE and ACP adapters.
use super::*;
use crate::agents::Backend;
use std::{
    io::Read,
    net::{Shutdown, TcpListener},
};

pub(super) fn serve_opencode(
    mut peer: Peer,
    state: Arc<Mutex<BackendState>>,
    stop: Arc<AtomicBool>,
    project: PathBuf,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture listener");
    listener.set_nonblocking(true).expect("configure listener");
    peer.write(
        json!({"url":format!("http://{}", listener.local_addr().expect("listener address"))}),
    );
    while !stop.load(Ordering::Relaxed) {
        let (stream, _) = match listener.accept() {
            Ok(pair) => pair,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
                continue;
            }
            Err(error) => panic!("fixture HTTP accept: {error}"),
        };
        stream
            .set_read_timeout(Some(WAIT))
            .expect("configure stream timeout");
        let mut reader = BufReader::new(stream);
        let mut head = String::new();
        reader.read_line(&mut head).expect("read request header");
        let path = head
            .split_whitespace()
            .nth(1)
            .expect("request path")
            .to_owned();
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read header line");
            if line == "\r\n" {
                break;
            }
            if let Some((key, value)) = line.split_once(':')
                && key.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse().expect("content length");
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).expect("read request body");
        state
            .lock()
            .expect("test lock should not be poisoned")
            .requests
            .push(json!({"http":head.trim(),"body":String::from_utf8(body).expect("UTF-8 request body")}));
        if path == "/api/event" {
            let mut stream = reader.into_inner();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n"
            )
            .expect("write event stream header");
            state
                .lock()
                .expect("test lock should not be poisoned")
                .event_stream = Some(stream);
            continue;
        }
        let data = if path.starts_with("/api/model?") {
            json!([{"id":"fixture-model","providerID":"fixture","name":"Fixture"}])
        } else if path.starts_with("/api/model/default?") {
            json!({"id":"fixture-model","providerID":"fixture"})
        } else if path.starts_with("/api/agent?")
            || path.starts_with("/api/command?")
            || path.contains("/message?")
        {
            json!([])
        } else if matches!(path.as_str(), "/api/session" | "/api/session/main-thread") {
            json!({"id":"main-thread","location":{"directory":project},"title":state.lock().expect("test lock should not be poisoned").name})
        } else {
            panic!("unhandled fixture HTTP request: {path}");
        };
        let body = json!({"data":data}).to_string();
        write!(reader.get_mut(), "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).expect("write fixture response");
    }
    if let Some(stream) = state
        .lock()
        .expect("test lock should not be poisoned")
        .event_stream
        .take()
    {
        let _ = stream.shutdown(Shutdown::Both);
    }
}

pub(super) fn serve_acp(mut peer: Peer, state: Arc<Mutex<BackendState>>, stop: Arc<AtomicBool>) {
    peer.reader
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(50)))
        .expect("configure peer timeout");
    while let Some(request) = read_request(&mut peer, &stop) {
        state
            .lock()
            .expect("test lock should not be poisoned")
            .requests
            .push(request.clone());
        let result = match request["method"].as_str().expect("request method") {
            "initialize" => {
                json!({"protocolVersion":1,"agentCapabilities":{},"authMethods":[{"id":"oauth-personal","name":"Sign in"},{"id":"cursor_login","name":"Sign in"}]})
            }
            "authenticate" | "session/close" | "session/set_mode" | "session/set_model" => {
                json!({})
            }
            "session/new" => {
                state.lock().expect("test lock should not be poisoned").main = Some(
                    peer.reader
                        .get_ref()
                        .try_clone()
                        .expect("clone peer socket"),
                );
                json!({"sessionId":"main-thread","models":{"currentModelId":"fixture-model","availableModels":[{"modelId":"fixture-model","name":"Fixture"}]}})
            }
            "cursor/list_available_models" => json!({"models":[]}),
            "session/prompt" => {
                write(
                    peer.reader.get_mut(),
                    json!({"jsonrpc":"2.0", "method":"session/update",
                        "params":{"sessionId":"main-thread", "update": {
                            "sessionUpdate":"agent_message_chunk", "content":{"type":"text","text":"Done"}
                        }}
                    }),
                );
                json!({"stopReason":"end_turn"})
            }
            _ => panic!("unhandled ACP request: {request}"),
        };
        write(
            peer.reader.get_mut(),
            json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
        );
    }
}

#[test]
fn opencode_resume_preserves_backend_title() {
    isolated_title(
        "title_native_tests::opencode_resume_preserves_backend_title",
        || {
            let s = Scenario::new(Backend::OpenCode, Some("Saved OpenCode title"), true);
            assert_eq!(
                s.name(),
                Some("Saved OpenCode title"),
                "HTTP session title must reach runtime state"
            );
        },
    );
}

#[test]
fn opencode_title_event_reaches_runtime_and_cache() {
    isolated_title(
        "title_native_tests::opencode_title_event_reaches_runtime_and_cache",
        || {
            let mut s = Scenario::new(Backend::OpenCode, None, false);
            s.until(|s| {
                s.backend
                    .state
                    .lock()
                    .expect("test lock should not be poisoned")
                    .event_stream
                    .is_some()
            });
            let event = json!({"type":"session.renamed","data":{"sessionID":"main-thread","title":"Native OpenCode title"}});
            writeln!(
                s.backend
                    .state
                    .lock()
                    .expect("test lock should not be poisoned")
                    .event_stream
                    .as_mut()
                    .expect("event stream"),
                "data: {event}\n"
            )
            .expect("write native title event");
            s.until(|s| s.name() == Some("Native OpenCode title"));
            assert_native_title_cached(&mut s, "Native OpenCode title");
        },
    );
}

#[test]
fn acp_and_cursor_title_events_reach_runtime_and_cache() {
    isolated_title(
        "title_native_tests::acp_and_cursor_title_events_reach_runtime_and_cache",
        || {
            for harness in [Backend::Antigravity, Backend::Cursor] {
                let mut s = Scenario::new(harness, None, false);
                write(
                    s.backend
                        .state
                        .lock()
                        .expect("test lock should not be poisoned")
                        .main
                        .as_mut()
                        .expect("main peer"),
                    json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"main-thread","update":{"sessionUpdate":"session_info_update","title":"Native ACP title"}}}),
                );
                s.until(|s| s.name() == Some("Native ACP title"));
                assert_native_title_cached(&mut s, "Native ACP title");
            }
        },
    );
}

fn assert_native_title_cached(s: &mut Scenario, expected: &str) {
    assert_eq!(s.cached_titles().last().map(String::as_str), Some(expected));
    assert_eq!(
        s.backend
            .state
            .lock()
            .expect("test lock should not be poisoned")
            .title_requests,
        0
    );
}
