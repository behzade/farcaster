use super::*;

#[test]
fn hex_busy_and_missing_capture_are_errors_not_empty_dictations() {
    for code in [
        "dictation-busy",
        "service-capture-unavailable",
        "microphone-lost",
    ] {
        let reply = format!("HTTP/1.1 409 Conflict\r\n\r\n{{\"code\":\"{code}\"}}");
        assert!(parse_response(reply.as_bytes()).unwrap_err().contains(code));
    }
    assert!(
        parse_response(b"HTTP/1.1 401 Unauthorized\r\n\r\n")
            .unwrap_err()
            .contains("401")
    );
}

#[test]
fn hex_rejects_incomplete_and_oversized_replies() {
    assert!(parse_response(b"HTTP/1.1 200 OK\r\n").is_err());
    assert!(parse_response(b"HTTP/1.1 200 OK\r\n\r\n{\"transcript\":").is_err());
    assert!(parse_response(&vec![0; 1024 * 1024 + 1]).is_err());
}

struct MockHex {
    endpoint: Endpoint,
    requests: mpsc::Receiver<&'static str>,
    server: std::thread::JoinHandle<()>,
}

impl MockHex {
    fn start(replies: Vec<(&'static str, u16, Value)>) -> Self {
        use std::io::{BufRead, BufReader};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = Endpoint {
            port: listener.local_addr().unwrap().port(),
            token: "test-token".into(),
            api_version: "2".into(),
        };
        let (send, requests) = mpsc::channel();
        let server = std::thread::spawn(move || {
            for (path, status, reply) in replies {
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                let stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < deadline, "missing {path}");
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("accept: {error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    request.push_str(&line);
                    if line == "\r\n" {
                        break;
                    }
                }
                assert!(
                    request.starts_with(&format!("{path} HTTP/1.1\r\n")),
                    "{request}"
                );
                assert!(request.contains("Authorization: Bearer test-token\r\n"));
                if path.contains("/7/") {
                    assert!(request.contains("X-Hex-Dictation-Token: recording-token\r\n"));
                }
                let length: usize = request
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                if path == "POST /dictations" {
                    assert_eq!(
                        serde_json::from_slice::<Value>(&body).unwrap()["source"],
                        "Farcaster"
                    );
                }
                let body = if status == 204 {
                    String::new()
                } else {
                    reply.to_string()
                };
                write!(reader.get_mut(), "HTTP/1.1 {status} Mock\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                let _ = send.send(path);
            }
        });
        Self {
            endpoint,
            requests,
            server,
        }
    }
}

fn capture_start() -> Vec<(&'static str, u16, Value)> {
    vec![
        ("GET /capabilities", 200, json!({"serviceCapture": true})),
        (
            "POST /dictations",
            201,
            json!({"id": 7, "ownerToken": "recording-token"}),
        ),
    ]
}

#[test]
fn release_before_hex_starts_still_finishes_and_returns_raw_text() {
    let mut replies = capture_start();
    replies.extend([
        (
            "POST /dictations/7/finish",
            200,
            json!({"transcript": "  Explain this\n", "durationMs": 250}),
        ),
        ("POST /dictations/7/cancel", 204, Value::Null),
    ]);
    let mock = MockHex::start(replies);
    let (send, commands) = mpsc::channel();
    let (updates, receiver) = async_channel::unbounded();
    send.send(Control::Finish).unwrap();
    record_with_endpoint(&mock.endpoint, commands, &updates).unwrap();
    assert!(matches!(receiver.try_recv().unwrap(), Event::Recording));
    assert!(
        matches!(receiver.try_recv().unwrap(), Event::Transcript(text) if text == "  Explain this\n")
    );
    mock.server.join().unwrap();
}

#[test]
fn dropping_dictation_after_heartbeat_cancels_only_its_owned_recording() {
    let mut replies = capture_start();
    replies.extend([
        ("POST /dictations/7/heartbeat", 204, Value::Null),
        ("POST /dictations/7/cancel", 204, Value::Null),
    ]);
    let mock = MockHex::start(replies);
    let (send, commands) = mpsc::channel();
    let dictation = Dictation(send);
    let (updates, receiver) = async_channel::unbounded();
    let worker =
        std::thread::spawn(move || record_with_endpoint(&mock.endpoint, commands, &updates));
    for path in [
        "GET /capabilities",
        "POST /dictations",
        "POST /dictations/7/heartbeat",
    ] {
        assert_eq!(
            mock.requests.recv_timeout(Duration::from_secs(10)).unwrap(),
            path
        );
    }
    drop(dictation);
    worker.join().unwrap().unwrap();
    mock.server.join().unwrap();
    assert!(matches!(receiver.try_recv().unwrap(), Event::Recording));
    assert!(
        receiver.try_recv().is_err(),
        "cancellation must not return a transcript"
    );
}

#[test]
fn linux_microphone_loss_is_an_error_and_releases_ownership() {
    let mut replies = capture_start();
    replies.extend([
        (
            "POST /dictations/7/finish",
            409,
            json!({"code": "microphone-lost"}),
        ),
        ("POST /dictations/7/cancel", 204, Value::Null),
    ]);
    let mock = MockHex::start(replies);
    let (send, commands) = mpsc::channel();
    let (updates, receiver) = async_channel::unbounded();
    send.send(Control::Finish).unwrap();
    assert!(
        record_with_endpoint(&mock.endpoint, commands, &updates)
            .unwrap_err()
            .contains("microphone-lost")
    );
    mock.server.join().unwrap();
    assert!(matches!(receiver.try_recv().unwrap(), Event::Recording));
    assert!(
        receiver.try_recv().is_err(),
        "do not submit an empty or partial transcript"
    );
}

#[test]
fn unavailable_capture_never_starts_a_dictation() {
    let mock = MockHex::start(vec![(
        "GET /capabilities",
        200,
        json!({"serviceCapture": false}),
    )]);
    let (_send, commands) = mpsc::channel();
    let (updates, receiver) = async_channel::unbounded();
    assert!(
        record_with_endpoint(&mock.endpoint, commands, &updates)
            .unwrap_err()
            .contains("3.3")
    );
    assert!(receiver.try_recv().is_err());
    mock.server.join().unwrap();
}
