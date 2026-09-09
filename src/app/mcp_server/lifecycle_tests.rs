use super::*;

#[test]
fn claude_protocol_can_initialize_and_list_tools_before_session_binding() {
    check_claude_tool_listing(None);
}

#[test]
#[ignore = "requires Node and FARCASTER_TEST_CLAUDE_SDK_PATH pointing to the installed sdk.mjs; no model calls"]
fn live_claude_sdk_accepts_farcaster_tools() {
    let sdk = std::env::var_os("FARCASTER_TEST_CLAUDE_SDK_PATH").expect("installed sdk.mjs path");
    check_claude_tool_listing(Some(std::path::Path::new(&sdk)));
}

fn check_claude_tool_listing(sdk: Option<&std::path::Path>) {
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;
    use std::time::Duration;

    let project = tempfile::tempdir().unwrap();
    let caller = crate::agents::CallerRegistry::shared().issue(
        project.path(),
        crate::agents::CallerProfile {
            backend: "claude".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    let (factories, backend) =
        crate::agents::worker_factories(crate::agents::AgentLaunchConfig::default());
    let workers =
        crate::agents::WorkerPool::new(factories, backend, project.path().into(), 1).unwrap();
    let (updates, _) = async_channel::bounded(1);
    let service = FarcasterMcp::new(
        project.path().join("state.db"),
        workers,
        updates,
        notices::NoticeBoard::default(),
    );
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = probe.local_addr().unwrap();
    drop(probe);
    let mut server = ServerState::new(service, true, &address.to_string()).unwrap();
    let request = |body: serde_json::Value| {
        let body = body.to_string();
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(stream,
            "POST /mcp HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2025-11-25\r\nfarcaster-caller: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            caller.token(), body.len(),
        ).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        serde_json::from_str::<serde_json::Value>(response.split_once("\r\n\r\n").unwrap().1)
            .unwrap()
    };
    let initialized = request(serde_json::json!({
        "jsonrpc":"2.0", "id":1, "method":"initialize",
        "params":{"protocolVersion":"2025-11-25", "capabilities":{},
            "clientInfo":{"name":"claude-sdk-test", "version":"1"}},
    }));
    assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
    let response =
        request(serde_json::json!({"jsonrpc":"2.0", "id":2, "method":"tools/list", "params":{}}));
    let tools = response["result"]["tools"].as_array().unwrap();
    for tool in tools {
        assert_eq!(
            tool["inputSchema"]["type"], "object",
            "{} input",
            tool["name"]
        );
        if let Some(schema) = tool.get("outputSchema") {
            assert_eq!(schema["type"], "object", "{} output", tool["name"]);
        }
    }
    assert!(tools.iter().any(|tool| tool["name"] == "worker_send"));
    if let Some(sdk) = sdk {
        let script = r#"
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
const {query} = await import(pathToFileURL(process.env.FARCASTER_TEST_CLAUDE_SDK_PATH));
const abortController = new AbortController();
const timer = setTimeout(() => abortController.abort(), 15000);
const session = query({
    prompt: (async function* () {
        await new Promise(resolve => abortController.signal.addEventListener('abort', resolve, {once: true}));
    })(),
    options: {cwd: process.cwd(), persistSession: false, abortController,
        mcpServers: {farcaster: {type: 'http', url: process.env.FARCASTER_TEST_MCP_URL,
            headers: {'farcaster-caller': process.env.FARCASTER_TEST_CALLER}}}}
});
try {
    while (!abortController.signal.aborted) {
        const server = (await session.mcpServerStatus()).find(server => server.name === 'farcaster');
        if (server?.status === 'failed') throw new Error(server.error);
        if (server?.status === 'connected') {
            assert.deepEqual(server.tools.map(tool => tool.name).sort(), JSON.parse(process.env.FARCASTER_TEST_TOOL_NAMES).sort());
            break;
        }
        await new Promise(resolve => setTimeout(resolve, 100));
    }
    assert.ok(!abortController.signal.aborted, 'MCP connection timed out');
} finally {
    clearTimeout(timer);
    abortController.abort();
    session.close();
}
"#;
        let expected = tools
            .iter()
            .map(|tool| tool["name"].clone())
            .collect::<Vec<_>>();
        let result = std::process::Command::new("node")
            .args(["--input-type=module", "-e", script])
            .current_dir(project.path())
            .env("FARCASTER_TEST_CLAUDE_SDK_PATH", sdk)
            .env("FARCASTER_TEST_MCP_URL", format!("http://{address}/mcp"))
            .env("FARCASTER_TEST_CALLER", caller.token())
            .env(
                "FARCASTER_TEST_TOOL_NAMES",
                serde_json::to_string(&expected).unwrap(),
            )
            .status()
            .unwrap();
        assert!(result.success(), "Claude SDK rejected Farcaster tools");
    }
    server.disable();
}

#[test]
fn disabled_server_leaves_the_port_free_and_can_be_reenabled() {
    let project = tempfile::tempdir().expect("project");
    let (factories, backend) =
        crate::agents::worker_factories(crate::agents::AgentLaunchConfig::default());
    let workers = crate::agents::WorkerPool::new(factories, backend, project.path().to_owned(), 1)
        .expect("workers");
    let (updates, _) = async_channel::bounded(1);
    let service = FarcasterMcp::new(
        project.path().join("state.db"),
        workers,
        updates,
        notices::NoticeBoard::default(),
    );
    let occupied = TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let address = occupied.local_addr().expect("address").to_string();
    assert!(ServerState::new(service.clone(), true, &address).is_err());
    let mut server =
        ServerState::new(service, false, &address).expect("disabled startup ignores occupied port");
    server.disable();
    assert!(server.enable(&address).is_err());
    assert!(server.running.is_none());
    drop(occupied);
    for _ in 0..2 {
        server.enable(&address).expect("enable server");
        assert!(
            TcpListener::bind(&address).is_err(),
            "enabled server owns the port"
        );
        server.disable();
        let probe = TcpListener::bind(&address).expect("disabled server releases port");
        drop(probe);
    }
}
