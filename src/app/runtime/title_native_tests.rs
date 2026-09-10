//! Native title metadata travels through real HTTP/SSE and ACP adapters.
use super::*;
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
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    peer.write(json!({"url":format!("http://{}", listener.local_addr().unwrap())}));
    while !stop.load(Ordering::Relaxed) {
        let (stream, _) = match listener.accept() {
            Ok(pair) => pair,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
                continue;
            }
            Err(error) => panic!("fixture HTTP accept: {error}"),
        };
        stream.set_read_timeout(Some(WAIT)).unwrap();
        let mut reader = BufReader::new(stream);
        let mut head = String::new();
        reader.read_line(&mut head).unwrap();
        let path = head.split_whitespace().nth(1).unwrap().to_owned();
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                if key.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap();
                }
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        state
            .lock()
            .unwrap()
            .requests
            .push(json!({"http":head.trim(),"body":String::from_utf8(body).unwrap()}));
        if path == "/api/event" {
            let mut stream = reader.into_inner();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n"
            )
            .unwrap();
            state.lock().unwrap().event_stream = Some(stream);
            continue;
        }
        let data = if path.starts_with("/api/model?") {
            json!([{"id":"fixture-model","providerID":"fixture","name":"Fixture"}])
        } else if path.starts_with("/api/agent?")
            || path.starts_with("/api/command?")
            || path.contains("/message?")
        {
            json!([])
        } else if matches!(path.as_str(), "/api/session" | "/api/session/main-thread") {
            json!({"id":"main-thread","location":{"directory":project},"title":state.lock().unwrap().name})
        } else {
            panic!("unhandled fixture HTTP request: {path}");
        };
        let body = json!({"data":data}).to_string();
        write!(reader.get_mut(), "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    }
    if let Some(stream) = state.lock().unwrap().event_stream.take() {
        let _ = stream.shutdown(Shutdown::Both);
    }
}

pub(super) fn serve_acp(mut peer: Peer, state: Arc<Mutex<BackendState>>, stop: Arc<AtomicBool>) {
    peer.reader
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    while let Some(request) = read_request(&mut peer, &stop) {
        state.lock().unwrap().requests.push(request.clone());
        let result = match request["method"].as_str().unwrap() {
            "initialize" => {
                json!({"protocolVersion":1,"agentCapabilities":{},"authMethods":[{"id":"oauth-personal","name":"Sign in"},{"id":"cursor_login","name":"Sign in"}]})
            }
            "authenticate" | "session/close" | "session/set_mode" | "session/set_model" => {
                json!({})
            }
            "session/new" => {
                state.lock().unwrap().main = Some(peer.reader.get_ref().try_clone().unwrap());
                json!({"sessionId":"main-thread","models":{"currentModelId":"fixture-model","availableModels":[{"modelId":"fixture-model","name":"Fixture"}]}})
            }
            "cursor/list_available_models" => json!({"models":[]}),
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
            let s = Scenario::new("opencode2", Some("Saved OpenCode title"), true);
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
            let mut s = Scenario::new("opencode2", None, false);
            s.until(|s| s.backend.state.lock().unwrap().event_stream.is_some());
            let event = json!({"type":"session.renamed","data":{"sessionID":"main-thread","title":"Native OpenCode title"}});
            writeln!(
                s.backend
                    .state
                    .lock()
                    .unwrap()
                    .event_stream
                    .as_mut()
                    .unwrap(),
                "data: {event}\n"
            )
            .unwrap();
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
            for harness in ["antigravity-acp", "cursor-cli"] {
                let mut s = Scenario::new(harness, None, false);
                write(
                    s.backend.state.lock().unwrap().main.as_mut().unwrap(),
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
    assert_eq!(s.backend.state.lock().unwrap().title_requests, 0);
}
