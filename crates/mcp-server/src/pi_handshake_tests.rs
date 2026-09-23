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
        std::sync::Arc::new(std::sync::Mutex::new(
            crate::storage::StateStore::open_at(&project.path().join("state.db")).expect("state"),
        )),
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
    assert_eq!(status, 200, "initialize failed: {response}");

    let (status, response) = request(
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
        &[("Mcp-Method", "notifications/initialized")],
    );
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
    assert_eq!(status, 200, "tools/call failed: {response}");
    server.disable();
}

#[test]
fn pi_extension_worker_send_returns_while_factory_is_starting() {
    isolated_pi_extension_test(
        "pi_extension_worker_send_returns_while_factory_is_starting",
        run_pi_extension_worker_send_test,
    );
}

fn run_pi_extension_worker_send_test() {
    use crate::agents::{
        CallerIdentity, CallerProfile, WorkerEvent, WorkerExecution, WorkerLaunch, WorkerProfile,
        WorkerProfiles, WorkerSession, WorkerSessionFactory,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Instant;

    struct DelayedFactory {
        started: Arc<AtomicUsize>,
        finished: Arc<AtomicUsize>,
    }

    struct DelayedSession(CallerIdentity);

    impl WorkerSessionFactory for DelayedFactory {
        fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
            self.started.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(8_250));
            let identity = crate::agents::CallerRegistry::shared().issue_as_with_access(
                &launch.project,
                CallerProfile {
                    backend: Backend::Pi,
                    provider: launch.provider.clone(),
                    model: launch.model.clone(),
                    effort: launch.effort.clone(),
                },
                None,
                launch.worker_id.clone(),
                launch.worker_name,
                launch.parent_worker_id,
                launch.access_mode,
            )?;
            identity.bind(format!("session-{}", launch.worker_id));
            self.finished.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(DelayedSession(identity)))
        }
    }

    impl WorkerSession for DelayedSession {
        fn send(&mut self, _: String, _: crate::agents::WorkerSendMode) -> Result<(), String> {
            Ok(())
        }
        fn respond(&mut self, _: crate::agents::WorkerInputResponse) -> Result<(), String> {
            Ok(())
        }
        fn abort(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn poll(&mut self) -> Option<WorkerEvent> {
            let _ = self.0.token();
            None
        }
        fn close(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    let project = tempfile::tempdir().expect("project");
    let started = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let factory: Arc<dyn WorkerSessionFactory> = Arc::new(DelayedFactory {
        started: started.clone(),
        finished: finished.clone(),
    });
    let workers = crate::agents::WorkerPool::new(
        std::collections::BTreeMap::from([(Backend::Pi, factory)]),
        Backend::Pi,
        project.path().into(),
        1,
    )
    .expect("workers");
    let caller = crate::agents::CallerRegistry::shared().issue_with_access(
        project.path(),
        CallerProfile {
            backend: Backend::Pi,
            provider: None,
            model: None,
            effort: None,
        },
        None,
        crate::agents::HarnessAccessMode::Full,
    );
    caller.bind("parent-session");

    let database = project.path().join("state.db");
    crate::storage::StateStore::open_at(&database)
        .expect("state store")
        .save_worker_profiles(&WorkerProfiles {
            profiles: vec![WorkerProfile {
                name: "test".into(),
                description: "Delayed Pi test worker".into(),
                limit: 10,
                enabled: true,
                models: vec![WorkerExecution {
                    harness: Backend::Pi,
                    provider: "test-provider".into(),
                    model: "test-model".into(),
                    effort: None,
                    service_tier: None,
                }],
            }],
            ..WorkerProfiles::default()
        })
        .expect("worker profiles");
    let observed_workers = workers.clone();
    let (updates, _) = async_channel::bounded(1);
    let service = FarcasterMcp::new(
        std::sync::Arc::new(std::sync::Mutex::new(
            crate::storage::StateStore::open_at(&database).expect("state"),
        )),
        workers,
        updates,
        notices::NoticeBoard::default(),
    );
    let address: std::net::SocketAddr = "127.0.0.1:18767".parse().expect("test address");
    let mut server =
        ServerState::new(service, true, &address.to_string()).expect("server should start");

    let extension = project.path().join("farcaster.mjs");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../agents/src/adapter/pi/farcaster.js"
        ),
        &extension,
    )
    .expect("copy Pi extension");
    let script = project.path().join("call-extension.mjs");
    std::fs::write(
        &script,
        r#"
const tools = [];
const pi = {
  on() {},
  registerCommand() {},
  registerTool(tool) { tools.push(tool); },
  appendEntry() {},
  sendMessage() {},
};
const extension = (await import(process.env.TEST_EXTENSION)).default;
await extension(pi);
const tool = tools.find(candidate => candidate.name === "farcaster_worker_send");
if (!tool) throw new Error("worker_send was not registered");
const result = await tool.execute(
  "call-1",
  {to: "delayed-child", message: "inspect", profile: "test"},
  new AbortController().signal,
);
if (result.isError) throw new Error(JSON.stringify(result));
if (result.details?.created !== true || result.details?.pending !== true) {
  throw new Error(`unexpected result: ${JSON.stringify(result)}`);
}
"#,
    )
    .expect("write Node probe");
    let began = Instant::now();
    let output = std::process::Command::new("node")
        .arg(&script)
        .env("TEST_EXTENSION", &extension)
        .env("FARCASTER_MCP_URL", format!("http://{address}/mcp"))
        .env("FARCASTER_MCP_CALLER", caller.token())
        .output()
        .expect("run Node Pi extension probe");
    assert!(
        output.status.success(),
        "Pi extension probe failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        began.elapsed() < Duration::from_secs(3),
        "worker_send waited for delayed factory startup"
    );
    let start_deadline = Instant::now() + Duration::from_secs(1);
    while started.load(Ordering::SeqCst) != 1 && Instant::now() < start_deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(started.load(Ordering::SeqCst), 1);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let running = observed_workers.snapshots().is_ok_and(|snapshots| {
            snapshots
                .iter()
                .any(|snapshot| snapshot.status == crate::agents::WorkerStatus::Running)
        });
        if finished.load(Ordering::SeqCst) == 1 && running {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "delayed worker did not finish setup"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    server.disable();
}

fn isolated_pi_extension_test(name: &str, run: impl FnOnce()) {
    const MARKER: &str = "FARCASTER_PI_EXTENSION_HTTP_TEST";
    let test_name = format!(
        "{}::{name}",
        module_path!().split_once("::").expect("test module path").1
    );
    if std::env::var(MARKER).as_deref() == Ok(test_name.as_str()) {
        run();
        return;
    }
    let temp = tempfile::tempdir().expect("isolated test directory");
    let log_path = temp.path().join("test.log");
    let log = std::fs::File::create(&log_path).expect("test log");
    let current_exe = std::env::current_exe().expect("current test executable");
    let mut command = std::process::Command::new(&current_exe);
    command
        .args(["--exact", &test_name, "--nocapture"])
        .env(MARKER, &test_name)
        .env("FARCASTER_PI_PATH", &current_exe)
        .env("FARCASTER_DATA_DIR", temp.path().join("data"))
        .stdout(log.try_clone().expect("clone test log"))
        .stderr(log);
    let mut child = command.spawn().expect("spawn isolated Pi extension test");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll isolated test") {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().expect("kill timed out isolated test");
            child.wait().expect("reap timed out isolated test");
            panic!(
                "Pi extension HTTP test timed out:\n{}",
                std::fs::read_to_string(&log_path).expect("read test log")
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = std::fs::read_to_string(log_path).expect("read test log");
    assert!(
        status.success(),
        "isolated Pi extension test failed:\n{output}"
    );
    assert!(
        output.contains("1 passed"),
        "isolated test did not run:\n{output}"
    );
}
