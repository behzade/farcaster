//! Isolated integration tests: supervisor -> real adapter -> fixture process -> snapshot.
use super::*;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    process::{Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};

#[path = "access_mode_lifecycle_tests.rs"]
mod access_mode_lifecycle_tests;

#[path = "code_tasks_tests.rs"]
mod code_tasks_tests;

#[path = "title_lifecycle_tests.rs"]
mod title_lifecycle_tests;

const WAIT: Duration = Duration::from_secs(3);
const CHILD_MARKER: &str = "FARCASTER_CATALOG_TEST_CHILD";

fn fixture_binary() -> &'static std::path::Path {
    static BINARY: OnceLock<tempfile::TempDir> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            let dir = tempfile::tempdir().expect("test operation should succeed");
            let output = Command::new("rustc")
                .arg("--edition=2024")
                .arg(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/catalog-agent.rs"
                ))
                .arg("-o")
                .arg(dir.path().join("agent"))
                .output()
                .expect("compile std-only catalog fixture with rustc");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            dir
        })
        .path()
}

/// Re-exec only this test before changing HOME/PATH. Parallel tests and real
/// credentials cannot leak into the supervisor's global environment or caches.
fn isolated(name: &str, backends: &[&str], run: impl FnOnce()) {
    isolated_with_env(name, backends, &[], run);
}

fn isolated_with_env(name: &str, backends: &[&str], env: &[(&str, &str)], run: impl FnOnce()) {
    let test_name = format!(
        "{}::{name}",
        module_path!()
            .split_once("::")
            .expect("test operation should succeed")
            .1
    );
    if std::env::var(CHILD_MARKER).as_deref() == Ok(test_name.as_str()) {
        run();
        return;
    }
    let dir = tempfile::Builder::new()
        .prefix("fc-cat-")
        .tempdir_in("/tmp")
        .expect("test operation should succeed");
    for backend in backends {
        fs::copy(fixture_binary().join("agent"), dir.path().join(backend))
            .expect("test operation should succeed");
    }
    // The ACP adapter checks that this sibling exists. The relay never invokes it.
    fs::copy(
        fixture_binary().join("agent"),
        dir.path().join("localharness_external"),
    )
    .expect("test operation should succeed");
    let log = fs::File::create(dir.path().join("test.log")).expect("test operation should succeed");
    let mut command = Command::new(std::env::current_exe().expect("test operation should succeed"));
    command
        .args(["--exact", &test_name, "--nocapture"])
        .env_clear()
        .env(CHILD_MARKER, &test_name)
        .env("HOME", dir.path())
        .env("PATH", dir.path())
        .env("FARCASTER_DATA_DIR", dir.path().join("data"))
        .env("SHELL", "/bin/sh")
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(log.try_clone().expect("test operation should succeed"))
        .stderr(log);
    command.envs(env.iter().copied());
    for (backend, variable) in [
        ("claude", "FARCASTER_CLAUDE_PATH"),
        ("antigravity-acp", "FARCASTER_ANTIGRAVITY_ACP_PATH"),
        ("cursor-cli", "FARCASTER_CURSOR_PATH"),
    ] {
        command.env(variable, dir.path().join(backend));
    }
    let mut child = command.spawn().expect("test operation should succeed");
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().expect("test operation should succeed") {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().expect("test operation should succeed");
            child.wait().expect("test operation should succeed");
            panic!(
                "catalog test watchdog expired:\n{}",
                fs::read_to_string(dir.path().join("test.log"))
                    .expect("test operation should succeed")
            );
        }
        thread::sleep(Duration::from_millis(5));
    };
    let output =
        fs::read_to_string(dir.path().join("test.log")).expect("test operation should succeed");
    assert!(status.success(), "isolated catalog test failed:\n{output}");
    assert!(
        output.contains("1 passed"),
        "child filter did not run exactly one test:\n{output}"
    );
}

struct Harness {
    listener: UnixListener,
    runtime: RuntimeHandle,
    project: PathBuf,
}

impl Harness {
    fn start() -> Self {
        let project = std::env::current_dir().expect("test operation should succeed");
        let listener = UnixListener::bind(project.join("control.sock"))
            .expect("test operation should succeed");
        listener
            .set_nonblocking(true)
            .expect("test operation should succeed");
        // Unlike RuntimeHandle::spawn_with, do not disable catalog discovery.
        let runtime = RuntimeHandle::spawn_with_configuration_refresh(
            project.clone(),
            crate::projects::DraftSession::with_id("pi".into(), "initial".into(), project.clone()),
            None,
            AgentLaunchConfig {
                session_locator_root: Some(project.join("session-locators")),
                ..AgentLaunchConfig::default()
            },
            true,
        );
        Self {
            listener,
            runtime,
            project,
        }
    }

    fn select(&self, backend: &str, id: &str) {
        self.runtime
            .send(RuntimeCommand::ResumeDraft {
                id: id.into(),
                harness: backend.into(),
                project: self.project.clone(),
            })
            .expect("test operation should succeed");
    }

    fn accept(&self, timeout: Duration) -> Option<Peer> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => return Some(Peer::new(stream)),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("fixture accept: {error}"),
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn snapshot(
        &self,
        backend: &str,
        matches: impl Fn(&RuntimeSnapshot) -> bool,
    ) -> Arc<RuntimeSnapshot> {
        let deadline = Instant::now() + WAIT;
        let mut last = None;
        loop {
            while let Ok(event) = self.runtime.try_recv() {
                if let RuntimeEvent::Snapshot { snapshot, .. } = event
                    && snapshot.harness == backend
                {
                    if matches(&snapshot) {
                        return snapshot;
                    }
                    last = Some(format!("{:?}", snapshot.configuration_status));
                }
            }
            assert!(
                Instant::now() < deadline,
                "no matching {backend} snapshot; last status: {last:?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}

struct Peer {
    reader: BufReader<UnixStream>,
    backend: String,
}

impl Peer {
    fn new(stream: UnixStream) -> Self {
        stream
            .set_nonblocking(false)
            .expect("test operation should succeed");
        stream
            .set_read_timeout(Some(WAIT))
            .expect("test operation should succeed");
        stream
            .set_write_timeout(Some(WAIT))
            .expect("test operation should succeed");
        let mut reader = BufReader::new(stream);
        let mut backend = String::new();
        assert!(
            reader
                .read_line(&mut backend)
                .expect("test operation should succeed")
                > 0
        );
        Self {
            reader,
            backend: backend.trim().into(),
        }
    }

    fn request(&mut self, method: &str) -> Value {
        let mut line = String::new();
        assert!(
            self.reader
                .read_line(&mut line)
                .unwrap_or_else(|error| panic!("{} waiting for {method}: {error}", self.backend))
                > 0,
            "{} closed before {method}",
            self.backend
        );
        let value: Value = serde_json::from_str(&line).expect("test operation should succeed");
        let actual = if self.backend == "claude" {
            &value["request"]["subtype"]
        } else {
            &value["method"]
        };
        assert_eq!(
            actual, method,
            "unexpected {} request: {value}",
            self.backend
        );
        value
    }

    fn write(&mut self, value: Value) {
        writeln!(self.reader.get_mut(), "{value}").expect("test operation should succeed");
    }

    fn reply(&mut self, request: &Value, result: Value) {
        self.write(if self.backend == "claude" {
            json!({"type":"control_response", "response":{
                "subtype":"success", "request_id":request["request_id"], "response":result
            }})
        } else {
            json!({"jsonrpc":"2.0", "id":request["id"], "result":result})
        });
    }

    fn complete_catalog(&mut self, fail: bool) {
        let request = self.request("initialize");
        if fail {
            self.write(if self.backend == "claude" {
                json!({"type":"control_response", "response":{
                    "subtype":"error", "request_id":request["request_id"], "error":"fixture unavailable"
                }})
            } else {
                json!({"jsonrpc":"2.0", "id":request["id"], "error":{
                    "code":-32603, "message":"fixture unavailable"
                }})
            });
            return;
        }
        if self.backend == "claude" {
            self.reply(&request, json!({
                "commands":[], "agents":[], "output_style":"default", "available_output_styles":[],
                "account":{}, "models":[{"value":"fixture-model", "displayName":"Fixture model", "description":"Test"}]
            }));
            return;
        }
        self.reply(&request, json!({"protocolVersion":1, "agentCapabilities":{},
            "authMethods":[{"id":"oauth-personal", "name":"Sign in"}, {"id":"cursor_login", "name":"Sign in"}]}));
        let request = self.request("authenticate");
        self.reply(&request, json!({}));
        let request = self.request("session/new");
        self.reply(&request, json!({"sessionId":"fixture-session", "models":{
            "currentModelId":"fixture-model", "availableModels":[{"modelId":"fixture-model", "name":"Fixture model"}]
        }}));
        if self.backend == "cursor-cli" {
            let request = self.request("cursor/list_available_models");
            self.reply(&request, json!({"models":[]}));
        }
        let request = self.request("session/close");
        self.reply(&request, json!({}));
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.reader.get_mut().shutdown(std::net::Shutdown::Both);
    }
}

fn round_trip(backend: &str) {
    let harness = Harness::start();
    harness.select(backend, "selected");
    let mut peer = harness
        .accept(WAIT)
        .expect("catalog did not launch fixture");
    assert_eq!(peer.backend, backend);
    peer.complete_catalog(false);
    let snapshot = harness.snapshot(backend, |s| {
        s.configuration_status == ConfigurationStatus::Loaded
    });
    assert!(
        snapshot
            .models
            .iter()
            .any(|model| model.id == "fixture-model")
    );
}

#[test]
fn antigravity_catalog_reaches_runtime_snapshot() {
    isolated(
        "antigravity_catalog_reaches_runtime_snapshot",
        &["antigravity-acp"],
        || round_trip("antigravity-acp"),
    );
}

#[test]
fn claude_cached_models_survive_failed_refresh_after_restart() {
    isolated(
        "claude_cached_models_survive_failed_refresh_after_restart",
        &["claude"],
        || {
            round_trip("claude");
            // Keep the same on-disk application state but replace the supervisor.
            fs::remove_file(
                std::env::current_dir()
                    .expect("test operation should succeed")
                    .join("control.sock"),
            )
            .expect("test operation should succeed");
            let harness = Harness::start();
            harness.select("claude", "restored");
            let mut peer = harness.accept(WAIT).expect("refresh did not start");
            assert_eq!(peer.backend, "claude");
            // The refreshed response is still held: these models must come from disk.
            let cached = harness.snapshot("claude", |s| {
                s.models.iter().any(|model| model.id == "fixture-model")
            });
            peer.complete_catalog(true);
            let failed = harness.snapshot("claude", |s| {
                matches!(&s.configuration_status, ConfigurationStatus::Failed(error) if error.contains("fixture unavailable"))
            });
            assert_eq!(failed.models, cached.models);
        },
    );
}

#[test]
fn stalled_acp_catalog_does_not_block_native_claude() {
    isolated(
        "stalled_acp_catalog_does_not_block_native_claude",
        &["antigravity-acp", "claude"],
        || {
            let harness = Harness::start();
            harness.select("antigravity-acp", "stalled");
            harness.select("claude", "healthy");
            let first = harness.accept(WAIT).expect("first backend did not start");
            let second = harness.accept(WAIT).expect("second backend did not start");
            let (mut stalled, mut healthy) = if first.backend == "antigravity-acp" {
                (first, second)
            } else {
                (second, first)
            };
            assert_eq!(stalled.backend, "antigravity-acp");
            assert_eq!(healthy.backend, "claude");
            stalled.request("initialize");
            healthy.complete_catalog(false);
            harness.snapshot("claude", |s| {
                s.configuration_status == ConfigurationStatus::Loaded
            });
            drop(stalled);
        },
    );
}

#[test]
fn startup_does_not_launch_unselected_catalogs() {
    isolated(
        "startup_does_not_launch_unselected_catalogs",
        &["claude", "antigravity-acp"],
        || {
            let harness = Harness::start();
            let launched = harness.accept(WAIT).map(|peer| peer.backend.clone());
            assert!(
                launched.is_none(),
                "startup launched {launched:?} without user selection"
            );
        },
    );
}

fn recovers_after_reselection(backend: &str) {
    let harness = Harness::start();
    harness.select(backend, "first");
    let mut peer = harness.accept(WAIT).expect("first catalog did not start");
    assert_eq!(peer.backend, backend);
    peer.complete_catalog(true);
    harness.snapshot(backend, |s| matches!(&s.configuration_status, ConfigurationStatus::Failed(error) if error.contains("fixture unavailable")));
    drop(peer);
    harness.select(backend, "second");
    let mut retry = harness
        .accept(WAIT)
        .expect("failed catalog never retried after reselection");
    assert_eq!(retry.backend, backend);
    retry.complete_catalog(false);
    let snapshot = harness.snapshot(backend, |s| {
        s.configuration_status == ConfigurationStatus::Loaded
    });
    assert!(
        snapshot
            .models
            .iter()
            .any(|model| model.id == "fixture-model")
    );
}

#[test]
fn claude_recovers_after_reselection() {
    isolated("claude_recovers_after_reselection", &["claude"], || {
        recovers_after_reselection("claude")
    });
}

#[test]
fn antigravity_recovers_after_reselection() {
    isolated(
        "antigravity_recovers_after_reselection",
        &["antigravity-acp"],
        || recovers_after_reselection("antigravity-acp"),
    );
}

#[test]
fn picker_request_retries_failed_catalog_and_coalesces_repeated_requests() {
    isolated(
        "picker_request_retries_failed_catalog_and_coalesces_repeated_requests",
        &["claude"],
        || {
            let harness = Harness::start();
            harness.select("claude", "selected");
            let mut peer = harness.accept(WAIT).expect("initial catalog did not start");
            peer.complete_catalog(true);
            harness.snapshot("claude", |s| {
                matches!(s.configuration_status, ConfigurationStatus::Failed(_))
            });
            drop(peer);
            for _ in 0..10 {
                harness
                    .runtime
                    .send(RuntimeCommand::LoadConfiguration {
                        harness: "claude".into(),
                        project: harness.project.clone(),
                    })
                    .expect("test operation should succeed");
            }
            let mut retry = harness.accept(WAIT).expect("picker request did not retry");
            harness.snapshot("claude", |s| {
                s.configuration_status == ConfigurationStatus::Loading
            });
            retry.complete_catalog(false);
            harness.snapshot("claude", |s| {
                s.configuration_status == ConfigurationStatus::Loaded
            });
            assert!(
                harness.accept(WAIT).is_none(),
                "repeated requests launched duplicate catalogs"
            );
        },
    );
}

#[test]
fn stalled_acp_catalog_does_not_block_another_backend() {
    isolated(
        "stalled_acp_catalog_does_not_block_another_backend",
        &["antigravity-acp", "cursor-cli"],
        || {
            let harness = Harness::start();
            harness.select("antigravity-acp", "stalled");
            let mut stalled = harness
                .accept(WAIT)
                .expect("first ACP backend did not start");
            // Receiving initialize proves this process is inside the catalog exchange.
            // Hold its response until the other backend has published models.
            stalled.request("initialize");
            let healthy = if stalled.backend == "cursor-cli" {
                "antigravity-acp"
            } else {
                "cursor-cli"
            };
            harness.select(healthy, "healthy");
            let mut peer = harness
                .accept(WAIT)
                .expect("one stalled ACP backend blocked the other");
            assert_eq!(peer.backend, healthy);
            peer.complete_catalog(false);
            harness.snapshot(healthy, |s| {
                s.configuration_status == ConfigurationStatus::Loaded
            });
            drop(stalled);
        },
    );
}
