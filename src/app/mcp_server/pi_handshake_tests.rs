// Diagnostic probe: replicate the injected Pi extension's exact MCP client sequence against the
// real server to locate the HTTP 400 the Pi extension observes at startup.
use super::*;
use crate::agents::Backend;
use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::time::Duration;

#[test]
fn steering_client_sequence_handshakes() {
    let project = tempfile::tempdir().expect("project");
    let caller = crate::agents::CallerRegistry::shared().issue(
        project.path(),
        crate::agents::CallerProfile {
            backend: Backend::Pi,
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    let (factories, backend) =
        crate::agents::worker_factories(crate::agents::AgentLaunchConfig::default());
    let workers = crate::agents::WorkerPool::new(factories, backend, project.path().into(), 1)
        .expect("workers");
    let (updates, _) = async_channel::bounded(1);
    let service = FarcasterMcp::new(
        project.path().join("state.db"),
        workers,
        updates,
        notices::NoticeBoard::default(),
    );
    let address: std::net::SocketAddr = "127.0.0.1:18766".parse().expect("probe address");
    let mut server =
        ServerState::new(service, true, &address.to_string()).expect("server should start");
    // Full 2026-07-28 client contract: per-request version header, SEP-2243
    // routing headers, and _meta request metadata on every non-initialize POST.
    let request = |body: String, extra_headers: &[(&str, &str)]| -> (u16, String) {
        let mut stream = TcpStream::connect(address).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        let mut raw = format!(
            "POST /mcp HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2026-07-28\r\nfarcaster-caller: {}\r\n",
            caller.token()
        );
        for (name, value) in extra_headers {
            raw.push_str(&format!("{name}: {value}\r\n"));
        }
        raw.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ));
        stream.write_all(raw.as_bytes()).expect("write request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        let status: u16 = response
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        (status, response)
    };

    let request_meta = serde_json::json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {}
    });

    let initialize = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2026-07-28",
            "capabilities": {},
            "clientInfo": {"name": "farcaster-pi", "version": "0.1.0"}
        }
    })
    .to_string();
    let (status, response) = request(initialize, &[]);
    println!("initialize: status={status}");
    assert_eq!(status, 200, "initialize failed: {response}");

    let (status, response) = request(
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
        &[("Mcp-Method", "notifications/initialized")],
    );
    println!("initialized notification: status={status}");
    assert!(
        status == 202 || status == 200,
        "notification failed: {response}"
    );

    let (status, response) = request(
        serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/list",
            "params": {"_meta": request_meta}
        })
        .to_string(),
        &[("Mcp-Method", "tools/list")],
    );
    println!("tools/list: status={status}");
    assert_eq!(status, 200, "tools/list failed: {response}");
    let payload: serde_json::Value =
        serde_json::from_str(response.split_once("\r\n\r\n").expect("body").1).expect("json");
    assert!(payload["result"]["tools"].as_array().expect("tools").len() >= 6);

    let (status, response) = request(
        serde_json::json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {
                "name": "workgraph_search",
                "arguments": {},
                "_meta": request_meta
            }
        })
        .to_string(),
        &[
            ("Mcp-Method", "tools/call"),
            ("Mcp-Name", "workgraph_search"),
        ],
    );
    println!("tools/call: status={status}");
    assert_eq!(status, 200, "tools/call failed: {response}");
    server.disable();
}
