use super::*;

#[test]
fn hex_busy_and_missing_capture_are_errors_not_empty_dictations() {
    for code in ["service-capture-busy", "service-capture-unavailable"] {
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

#[test]
fn release_before_hex_starts_still_finishes_and_returns_text() {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Endpoint {
        port: listener.local_addr().unwrap().port(),
        token: "test-token".into(),
        api_version: "2".into(),
    };
    let server = std::thread::spawn(move || {
        for (path, reply) in [
            ("GET /capabilities", json!({"serviceCapture": true})),
            (
                "POST /dictations",
                json!({"id": 7, "ownerToken": "recording-token"}),
            ),
            (
                "POST /dictations/7/finish",
                json!({"transcript": "Explain this", "durationMs": 250}),
            ),
            ("POST /dictations/7/cancel", Value::Null),
        ] {
            let (stream, _) = listener.accept().unwrap();
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
            assert!(request.starts_with(path), "{request}");
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
            let body = reply.to_string();
            write!(
                reader.get_mut(),
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    let (send, commands) = mpsc::channel();
    let (updates, receiver) = async_channel::unbounded();
    send.send(Control::Finish).unwrap();
    record_with_endpoint(&endpoint, commands, &updates).unwrap();
    assert!(matches!(receiver.try_recv().unwrap(), Event::Recording));
    assert!(
        matches!(receiver.try_recv().unwrap(), Event::Transcript(text) if text == "Explain this")
    );
    server.join().unwrap();
}
