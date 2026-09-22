use super::*;
use std::time::Instant;

pub(in crate::adapter) fn request(url: &str, session: &str) -> thread::JoinHandle<String> {
    let url = url::Url::parse(url).expect("boundary URL");
    let body = serde_json::json!({"session_id":session}).to_string();
    thread::spawn(move || {
        let mut stream =
            TcpStream::connect(("127.0.0.1", url.port().expect("port"))).expect("connect hook");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        write!(
            stream,
            "POST {} HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
            url.path(),
            body.len()
        )
        .expect("write hook");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read release");
        response
    })
}

pub(in crate::adapter) fn receive(gate: &PromptBoundary) -> Boundary {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(boundary) = gate.poll() {
            return boundary;
        }
        assert!(Instant::now() < deadline, "hook did not reach gate");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn empty_boundary_returns_without_owner_polling() {
    let gate = PromptBoundary::new(None).expect("gate");
    let response = request(&gate.url, "parent").join().expect("request thread");
    assert!(response.ends_with("{}"));
    assert!(gate.poll().is_none());
}

#[test]
fn enabled_boundary_waits_for_explicit_release() {
    let gate = PromptBoundary::new(None).expect("gate");
    gate.enable(true);
    let response = request(&gate.url, "parent");
    let boundary = receive(&gate);
    assert_eq!(boundary.session_id, "parent");
    assert!(!response.is_finished());
    boundary.release(true);
    assert!(
        response
            .join()
            .expect("request thread")
            .contains("\"continue\":false")
    );
}

#[test]
fn closing_gate_releases_unclaimed_hooks() {
    let mut gate = PromptBoundary::new(None).expect("gate");
    gate.enable(true);
    let response = request(&gate.url, "parent");
    // Requeue a fully parsed real connection so shutdown cannot race accept.
    let boundary = receive(&gate);
    let (sender, incoming) = mpsc::sync_channel(1);
    sender.send(boundary).expect("requeue hook");
    gate.incoming = incoming;
    assert!(!response.is_finished());
    drop(gate);
    assert!(response.join().expect("request thread").ends_with("{}"));
}
