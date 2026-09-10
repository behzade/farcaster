//! Runtime -> real Pi/Codex adapter -> isolated subprocess -> runtime metadata.
//! Only protocol peers are scripted; title requests and rename requests run normally.
use super::*;
use crate::app::runtime::tests::owner_without_process;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

#[path = "title_native_tests.rs"]
mod title_native_tests;

const GENERATED: &str = "Inspect archive contents";

fn isolated_title(name: &str, run: impl FnOnce()) {
    isolated_with_env(
        &format!("title_lifecycle_tests::{name}"),
        &["pi", "codex", "opencode2", "cursor-cli", "antigravity-acp"],
        &[("FARCASTER_FIXTURE_REPORT_MODE", "1")],
        run,
    );
}

#[derive(Default)]
struct BackendState {
    name: Option<String>,
    messages: usize,
    title_requests: usize,
    release_title: bool,
    empty_title: bool,
    reject_rename: bool,
    requests: Vec<Value>,
    main: Option<UnixStream>,
    event_stream: Option<std::net::TcpStream>,
}

struct Backend {
    state: Arc<Mutex<BackendState>>,
    stop: Arc<AtomicBool>,
    server: Option<thread::JoinHandle<()>>,
}

impl Backend {
    fn start() -> Self {
        let project = std::env::current_dir().unwrap();
        let socket = project.join("control.sock");
        if socket.exists() {
            fs::remove_file(&socket).unwrap();
        }
        let listener = UnixListener::bind(socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let state = Arc::new(Mutex::new(BackendState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let shared = state.clone();
        let stopped = stop.clone();
        let server = thread::spawn(move || {
            let mut peers = Vec::new();
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let state = shared.clone();
                        let stop = stopped.clone();
                        let project = project.clone();
                        peers.push(thread::spawn(move || serve(stream, state, stop, project)));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("accept fixture: {error}"),
                }
            }
            for peer in peers {
                peer.join().unwrap();
            }
        });
        Self {
            state,
            stop,
            server: Some(server),
        }
    }

    fn rename_requests(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter_map(|request| {
                match request["method"].as_str().or(request["type"].as_str()) {
                    Some("thread/name/set") => request["params"]["name"].as_str(),
                    Some("set_session_name") => request["name"].as_str(),
                    _ => None,
                }
                .map(str::to_owned)
            })
            .collect()
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let result = self.server.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}

fn write(peer: &mut UnixStream, value: Value) {
    if let Err(error) = writeln!(peer, "{value}") {
        // Runtime shutdown can close a pipe while a reply is in flight.
        assert!(
            matches!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
            ),
            "fixture write: {error}"
        );
    }
}

fn title_output(state: &Mutex<BackendState>, stop: &AtomicBool) -> Option<String> {
    state.lock().unwrap().title_requests += 1;
    while !stop.load(Ordering::Relaxed) {
        let state = state.lock().unwrap();
        if state.release_title {
            return Some(if state.empty_title { "" } else { GENERATED }.into());
        }
        drop(state);
        thread::sleep(Duration::from_millis(2));
    }
    None
}

fn read_request(peer: &mut Peer, stop: &AtomicBool) -> Option<Value> {
    while !stop.load(Ordering::Relaxed) {
        let mut line = String::new();
        match peer.reader.read_line(&mut line) {
            Ok(0) => return None,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(error) => panic!("read fixture: {error}"),
        }
        return Some(serde_json::from_str(&line).unwrap());
    }
    None
}

fn serve(
    stream: UnixStream,
    state: Arc<Mutex<BackendState>>,
    stop: Arc<AtomicBool>,
    project: PathBuf,
) {
    let mut peer = Peer::new(stream);
    let mut mode = String::new();
    peer.reader.read_line(&mut mode).unwrap();
    if peer.backend == "opencode2" {
        title_native_tests::serve_opencode(peer, state, stop, project);
        return;
    }
    if matches!(peer.backend.as_str(), "cursor-cli" | "antigravity-acp") {
        title_native_tests::serve_acp(peer, state, stop);
        return;
    }
    if mode.trim() == "print" {
        if let Some(title) = title_output(&state, &stop) {
            writeln!(peer.reader.get_mut(), "{title}").unwrap();
        }
        return;
    }
    peer.reader
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let mut ephemeral = false;
    while let Some(request) = read_request(&mut peer, &stop) {
        let method = request["method"]
            .as_str()
            .or(request["type"].as_str())
            .unwrap();
        state.lock().unwrap().requests.push(request.clone());
        if method == "initialized" {
            continue;
        }
        let mut failure = false;
        let result = match method {
            "initialize" => {
                json!({"userAgent":"fixture", "codexHome":project, "platformFamily":"unix", "platformOs":"linux"})
            }
            "model/list" => json!({"data":[{"id":"fixture-model", "modelProvider":"openai"}]}),
            "collaborationMode/list" | "skills/list" | "thread/list" => json!({"data":[]}),
            "thread/read" | "thread/resume" | "thread/fork" | "thread/start" => {
                ephemeral = request["params"]["ephemeral"].as_bool().unwrap_or(false);
                let mut state = state.lock().unwrap();
                if method != "thread/read" && !ephemeral {
                    state.main = Some(peer.reader.get_ref().try_clone().unwrap());
                }
                json!({"thread":{"id":if ephemeral {"title-thread"} else {"main-thread"}, "cwd":project, "name":state.name, "turns":[]}})
            }
            "thread/goal/get" => json!({"goal":null}),
            "turn/start" => json!({"turn":{"id":"turn-1", "status":"inProgress"}}),
            "thread/name/set" | "set_session_name" => {
                let mut state = state.lock().unwrap();
                failure = state.reject_rename;
                if !failure {
                    state.name = request["params"]["name"]
                        .as_str()
                        .or(request["name"].as_str())
                        .map(str::to_owned);
                }
                json!({})
            }
            "get_state" => {
                let state = state.lock().unwrap();
                json!({"sessionId":"main-thread", "sessionFile":project.join("main.jsonl"),
                    "sessionName":state.name, "isStreaming":false, "isCompacting":false,
                    "autoCompactionEnabled":true, "messageCount":state.messages, "pendingMessageCount":0})
            }
            "get_entries" => json!({"entries":[], "leafId":null}),
            "get_messages" => json!({"messages":[]}),
            "get_available_models" => json!({"models":[]}),
            "get_available_thinking_levels" => json!({"levels":["off"]}),
            "get_commands" => json!({"commands":[]}),
            "get_modes" => json!({"modes":[]}),
            "set_steering_mode" | "get_session_stats" | "prompt" | "abort" => json!({}),
            _ => panic!("unhandled fixture request: {request}"),
        };
        if peer.backend == "pi" {
            write(
                peer.reader.get_mut(),
                json!({"type":"response", "id":request["id"], "command":method,
                "success":!failure, "data":result, "error":if failure {Some("rename denied")} else {None}}),
            );
        } else if failure {
            write(
                peer.reader.get_mut(),
                json!({"id":request["id"], "error":{"code":-32000,"message":"rename denied"}}),
            );
        } else {
            peer.reply(&request, result);
        }
        if method == "turn/start" {
            let text = if ephemeral {
                let Some(title) = title_output(&state, &stop) else {
                    return;
                };
                title
            } else {
                "Done".into()
            };
            let id = if ephemeral {
                "title-thread"
            } else {
                "main-thread"
            };
            for (method, params) in [
                (
                    "turn/started",
                    json!({"threadId":id,"turn":{"id":"turn-1","status":"inProgress"}}),
                ),
                (
                    "item/completed",
                    json!({"threadId":id,"item":{"id":"answer","type":"agentMessage","text":text}}),
                ),
                (
                    "turn/completed",
                    json!({"threadId":id,"turn":{"id":"turn-1","status":"completed"}}),
                ),
            ] {
                write(
                    peer.reader.get_mut(),
                    json!({"method":method,"params":params}),
                );
            }
        } else if method == "prompt" {
            state.lock().unwrap().messages += 2;
            write(peer.reader.get_mut(), json!({"type":"agent_start"}));
            write(
                peer.reader.get_mut(),
                json!({"type":"agent_end","messages":[]}),
            );
            write(peer.reader.get_mut(), json!({"type":"agent_settled"}));
        }
    }
}

struct Scenario {
    owner: RuntimeOwner,
    incoming_events: mpsc::Receiver<RuntimeEvent>,
    events: Vec<RuntimeEvent>,
    backend: Backend,
}

impl Scenario {
    fn new(harness: &str, name: Option<&str>, resume: bool) -> Self {
        let backend = Backend::start();
        backend.state.lock().unwrap().name = name.map(str::to_owned);
        let project = std::env::current_dir().unwrap();
        fs::write(project.join("main.jsonl"), "").unwrap();
        let (mut owner, incoming_events) = owner_without_process(project.clone());
        owner.harness = harness.into();
        owner.state = Some(StateStore::open().unwrap());
        owner.process_command = AgentLaunchConfig {
            program: project.join("pi"),
            session_locator_root: Some(project.join("locators")),
            ..AgentLaunchConfig::default()
        };
        let path = if harness == "pi" {
            project.join("main.jsonl")
        } else {
            project.join("locators").join(harness).join("main-thread")
        };
        owner.start_process(resume.then_some(path));
        let mut scenario = Self {
            owner,
            incoming_events,
            events: Vec::new(),
            backend,
        };
        scenario.until(|s| s.owner.startup_state_loaded && s.owner.startup_history_loaded);
        scenario
    }

    fn pump(&mut self) {
        while let Some(event) = self.owner.process.as_mut().and_then(|p| p.poll()) {
            self.owner.apply_process_item(event);
        }
        self.events.extend(self.incoming_events.try_iter());
    }

    fn until(&mut self, matches: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            self.pump();
            if matches(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{} runtime did not reach expected state: {}; transcript: {:?}",
                self.owner.harness,
                self.owner.snapshot.status,
                self.owner
                    .snapshot
                    .conversation
                    .items
                    .iter()
                    .map(|item| &item.text)
                    .collect::<Vec<_>>()
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn prompt(&mut self) {
        let before = self.events.len();
        self.owner.send_prompt(
            format!(
                "session:{}",
                self.owner.active_session.as_ref().unwrap().display()
            ),
            PromptMode::Normal,
            "Inspect this archive".into(),
            Vec::new(),
            false,
        );
        self.until(|s| {
            s.events[before..]
                .iter()
                .any(|event| matches!(event, RuntimeEvent::PromptResult { .. }))
        });
        assert!(
            self.events[before..]
                .iter()
                .any(|event| matches!(event, RuntimeEvent::PromptResult { accepted: true, .. })),
            "{} fixture prompt was rejected: {}",
            self.owner.harness,
            self.owner.snapshot.status
        );
        self.until(|s| !s.owner.normal_prompt_in_flight && !s.owner.snapshot.conversation.running);
    }

    fn generate(&mut self) {
        self.prompt();
        self.until(|s| s.backend.state.lock().unwrap().title_requests == 1);
    }

    fn title_result(&mut self) -> SessionTitleResult {
        self.backend.state.lock().unwrap().release_title = true;
        self.owner
            .title_generation
            .receiver
            .recv_timeout(WAIT)
            .expect("real title process must finish")
    }

    fn finish_title(&mut self) {
        let result = self.title_result();
        assert!(
            result.result.is_ok(),
            "title fixture failed: {:?}",
            result.result
        );
        self.owner.apply_generated_session_title(result);
        self.pump();
    }

    fn cached_titles(&mut self) -> Vec<String> {
        self.events
            .drain(..)
            .filter_map(|event| match event {
                RuntimeEvent::SessionMetadata(metadata) => Some(
                    self.owner
                        .state
                        .as_mut()
                        .unwrap()
                        .update_session_metadata(&metadata)
                        .unwrap()
                        .title,
                ),
                _ => None,
            })
            .collect()
    }

    fn name(&self) -> Option<&str> {
        self.owner
            .active_snapshot()
            .session
            .as_ref()
            .and_then(|s| s.session_name.as_deref())
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        if let Some(mut process) = self.owner.process.take() {
            let _ = process.close();
        }
    }
}

#[test]
fn fresh_sessions_generate_and_persist_one_title() {
    isolated_title("fresh_sessions_generate_and_persist_one_title", || {
        for harness in ["pi", "codex-cli"] {
            let mut s = Scenario::new(harness, None, false);
            s.generate();
            s.finish_title();
            s.until(|s| {
                s.backend.state.lock().unwrap().name.as_deref() == Some(GENERATED)
                    && s.name() == Some(GENERATED)
            });
            assert_eq!(s.name(), Some(GENERATED), "{harness}");
            assert_eq!(s.backend.rename_requests(), [GENERATED]);
            assert_eq!(
                s.cached_titles().last().map(String::as_str),
                Some(GENERATED)
            );
            s.prompt();
            assert_eq!(s.backend.state.lock().unwrap().title_requests, 1);
            assert!(!s.owner.title_generation.in_flight);
        }
    });
}

#[test]
fn waking_existing_sessions_does_not_generate_titles() {
    isolated_title("waking_existing_sessions_does_not_generate_titles", || {
        for harness in ["pi", "codex-cli"] {
            for name in [None, Some("Keep existing title")] {
                let mut s = Scenario::new(harness, name, true);
                s.prompt();
                assert!(
                    !s.owner.title_generation.in_flight,
                    "{harness} resumed session started generation"
                );
                assert_eq!(s.backend.state.lock().unwrap().title_requests, 0);
                assert!(s.backend.rename_requests().is_empty());
            }
        }
    });
}

#[test]
fn codex_resume_preserves_backend_title() {
    isolated_title("codex_resume_preserves_backend_title", || {
        let s = Scenario::new("codex-cli", Some("Keep existing title"), true);
        assert_eq!(
            s.name(),
            Some("Keep existing title"),
            "resume response name must reach runtime state"
        );
    });
}

#[test]
fn codex_native_title_wins_over_pending_generation() {
    isolated_title("codex_native_title_wins_over_pending_generation", || {
        let mut s = Scenario::new("codex-cli", None, false);
        s.generate();
        {
            let mut backend = s.backend.state.lock().unwrap();
            backend.name = Some("Native title".into());
            write(
                backend.main.as_mut().unwrap(),
                json!({"method":"thread/name/updated", "params":{"threadId":"main-thread","threadName":"Native title"}}),
            );
            write(
                backend.main.as_mut().unwrap(),
                json!({"method":"turn/started", "params":{"threadId":"main-thread","turn":{"id":"native-turn","status":"inProgress"}}}),
            );
        }
        // Seeing the following turn event proves the preceding name event was read.
        s.until(|s| s.owner.snapshot.conversation.running);
        s.finish_title();
        assert!(
            s.backend.rename_requests().is_empty(),
            "generated title overwrote a native title: {:?}",
            s.backend.rename_requests()
        );
        assert_eq!(s.name(), Some("Native title"));
    });
}

fn rejected_rename(harness: &str) {
    let mut s = Scenario::new(harness, None, false);
    s.generate();
    s.backend.state.lock().unwrap().reject_rename = true;
    s.finish_title();
    s.until(|s| {
        s.owner
            .snapshot
            .conversation
            .items
            .iter()
            .any(|item| item.text.contains("rename denied"))
    });
    assert_eq!(s.backend.rename_requests(), [GENERATED]);
    assert_eq!(s.backend.state.lock().unwrap().name, None);
    let cached_titles = s.cached_titles();
    assert!(
        !cached_titles.iter().any(|title| title == GENERATED),
        "{harness} cached a title the backend rejected: {cached_titles:?}"
    );
    assert_eq!(s.name(), None);
}

#[test]
fn pi_failed_rename_does_not_publish_generated_title() {
    isolated_title("pi_failed_rename_does_not_publish_generated_title", || {
        rejected_rename("pi")
    });
}

#[test]
fn codex_failed_rename_does_not_publish_generated_title() {
    isolated_title(
        "codex_failed_rename_does_not_publish_generated_title",
        || rejected_rename("codex-cli"),
    );
}

#[test]
fn manual_rename_wins_over_pending_generation() {
    isolated_title("manual_rename_wins_over_pending_generation", || {
        for harness in ["pi", "codex-cli"] {
            let mut s = Scenario::new(harness, None, false);
            s.generate();
            s.owner
                .apply_command(RuntimeCommand::SetSessionName("My title".into()));
            s.until(|s| s.backend.state.lock().unwrap().name.as_deref() == Some("My title"));
            s.finish_title();
            assert_eq!(s.backend.rename_requests(), ["My title"], "{harness}");
        }
    });
}

#[test]
fn replaced_process_rejects_late_generated_title() {
    isolated_title("replaced_process_rejects_late_generated_title", || {
        for harness in ["pi", "codex-cli"] {
            let mut s = Scenario::new(harness, None, false);
            s.generate();
            s.owner.reset_process_runtime();
            s.finish_title();
            assert!(s.backend.rename_requests().is_empty(), "{harness}");
        }
    });
}

#[test]
fn empty_generator_output_does_not_rename_session() {
    isolated_title("empty_generator_output_does_not_rename_session", || {
        for harness in ["pi", "codex-cli"] {
            let mut s = Scenario::new(harness, None, false);
            s.backend.state.lock().unwrap().empty_title = true;
            s.generate();
            let result = s.title_result();
            assert!(result.result.is_err(), "empty title should fail validation");
            s.owner.apply_generated_session_title(result);
            assert!(s.backend.rename_requests().is_empty(), "{harness}");
            assert_eq!(s.name(), None);
        }
    });
}

#[test]
fn backend_named_new_codex_session_does_not_start_another_title() {
    isolated_title(
        "backend_named_new_codex_session_does_not_start_another_title",
        || {
            let mut s = Scenario::new("codex-cli", Some("Backend supplied title"), false);
            s.prompt();
            assert!(
                !s.owner.title_generation.in_flight,
                "thread/start already supplied a name, but Farcaster started another title request"
            );
            assert!(s.backend.rename_requests().is_empty());
        },
    );
}

#[test]
fn pi_backend_title_wins_over_pending_generation() {
    isolated_title("pi_backend_title_wins_over_pending_generation", || {
        let mut s = Scenario::new("pi", None, false);
        s.generate();
        s.backend.state.lock().unwrap().name = Some("Backend title".into());
        s.owner.send(SessionCommand::LoadState);
        s.until(|s| s.name() == Some("Backend title"));
        s.finish_title();
        assert!(s.backend.rename_requests().is_empty());
        assert_eq!(s.name(), Some("Backend title"));
    });
}

#[test]
fn forks_do_not_generate_replacement_titles() {
    isolated_title("forks_do_not_generate_replacement_titles", || {
        for harness in ["pi", "codex-cli"] {
            let mut s = Scenario::new(harness, None, true);
            let source = s.owner.active_session.clone().unwrap();
            s.owner.start_process_from(None, Some(source), false);
            s.until(|s| s.owner.startup_state_loaded && s.owner.startup_history_loaded);
            s.prompt();
            assert!(
                !s.owner.title_generation.in_flight,
                "{harness} fork started title inference"
            );
            assert_eq!(s.backend.state.lock().unwrap().title_requests, 0);
            assert!(s.backend.rename_requests().is_empty());
        }
    });
}
