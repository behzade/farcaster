use super::super::client::{Client, frame};
use super::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

type Requests = Arc<Mutex<Vec<(String, Value)>>>;
struct Fixture {
    project: std::path::PathBuf,
    client: Client,
    requests: Requests,
    stopped: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    fn new(hold: bool, truncate: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
        let address = listener.local_addr().expect("address");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let requests: Requests = Default::default();
        let recorded = requests.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let runs = Arc::new(AtomicUsize::new(0));
        let task = thread::spawn(move || {
            let mut handlers = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let Ok((socket, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                };
                let recorded = recorded.clone();
                let cancelled = cancelled.clone();
                let runs = runs.clone();
                let stop = stop.clone();
                handlers.push(thread::spawn(move || {
                    serve(socket, recorded, cancelled, runs, stop, hold, truncate)
                }));
            }
            for handler in handlers {
                handler.join().expect("fixture handler");
            }
        });
        Self {
            project: std::path::PathBuf::from(format!("/fixture/{}", uuid::Uuid::new_v4())),
            client: Client::new(&format!("http://{address}"), "fixture-token".into())
                .expect("client"),
            requests,
            stopped,
            thread: Some(task),
        }
    }
    fn worker(&self, resume: Option<&str>) -> Worker {
        Worker::from_bridge(
            Bridge::fixture(self.client.clone(), Some("fixture-key".into())),
            &AgentLaunchConfig::default(),
            self.project.as_path(),
            resume,
            Some("caller-token"),
            None,
        )
        .expect("worker")
        .0
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.thread
            .take()
            .expect("fixture task")
            .join()
            .expect("fixture server");
    }
}
fn serve(
    mut socket: TcpStream,
    requests: Requests,
    cancelled: Arc<AtomicBool>,
    runs: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    hold: bool,
    truncate: bool,
) {
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        socket.read_exact(&mut byte).expect("request header");
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).expect("header UTF8");
    assert!(head.contains("Authorization: Bearer fixture-token\r\n"));
    let method = head
        .lines()
        .next()
        .expect("request")
        .split_whitespace()
        .nth(1)
        .expect("path")
        .rsplit('/')
        .next()
        .expect("method")
        .to_owned();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .expect("content length")
        .parse()
        .expect("length");
    let mut body = vec![0; length];
    socket.read_exact(&mut body).expect("request body");
    let value: Value = serde_json::from_slice(if method == "Send" { &body[5..] } else { &body })
        .expect("request JSON");
    requests
        .lock()
        .expect("record requests")
        .push((method.clone(), value.clone()));
    if method == "Send" {
        let run = format!("run-{}", runs.fetch_add(1, Ordering::Relaxed));
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/connect+json\r\nConnection: close\r\n\r\n").expect("stream response");
        if hold {
            thread::sleep(Duration::from_millis(50));
        }
        let send = |socket: &mut TcpStream, value: Value| {
            socket
                .write_all(&frame(&serde_json::to_vec(&value).expect("event JSON")))
                .is_ok()
        };
        if !send(
            &mut socket,
            json!({"sdkMessage":{"type":"system","message":{"run_id":run,"agent_id":"sdk-agent"}}}),
        ) {
            return;
        }
        if hold {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !cancelled.load(Ordering::Relaxed)
                && !stop.load(Ordering::Relaxed)
                && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(2));
            }
        } else {
            send(&mut socket, json!({}));
            send(
                &mut socket,
                json!({"interactionUpdate":{"type":"text-delta","update":{"text":"hello"}}}),
            );
            for fragment in ["hel", "lo"] {
                send(
                    &mut socket,
                    json!({"sdkMessage":{"type":"assistant","message":{"run_id":run,"message":{"content":[{"type":"text","text":fragment}]}}}}),
                );
            }
        }
        if truncate {
            return;
        }
        send(
            &mut socket,
            json!({"result":{"runId":run,"status":if hold {"RUN_LIFECYCLE_STATUS_CANCELLED"} else {"RUN_LIFECYCLE_STATUS_FINISHED"},"result":{"result":if hold {""} else {"hello"},"usage":{"inputTokens":"2","outputTokens":"1"}}}}),
        );
        send(&mut socket, json!({"done":{"runId":run}}));
        let mut end = frame(b"{}");
        end[0] = 2;
        let _ = socket.write_all(&end);
        return;
    }
    let response = match method.as_str() {
        "ListModels" => {
            json!({"items":[{"id":"test-model","parameters":[{"id":"fast","values":[{"value":"false"},{"value":"true"}]}]}]})
        }
        "CreateAgent" | "ResumeAgent" => json!({"agentId":"sdk-agent"}),
        "CancelRun" => {
            cancelled.store(true, Ordering::Relaxed);
            json!({})
        }
        "Shutdown" => json!({}),
        "SteerRun" => {
            json!({"outcome": if value["text"] == "fallback" { "revert_to_followup" } else { "complete_delivered" }})
        }
        "ListRuns" => {
            json!({"items":[{"model":{"id":"test-model","params":[{"id":"fast","value":"true"}]}}]})
        }
        "ListAgentMessages" => json!({"messages":[
            {"uuid":"sdk-agent:0","type":"user","message":{"turn":{"case":"agentConversationTurn","value":{
                "userMessage":{"text":"Earlier prompt"},
                "steps":[
                    {"message":{"case":"thinkingMessage","value":{"text":"Considering the prompt","durationMs":498}}},
                    {"message":{"case":"assistantMessage","value":{"text":"Earlier reply"}}}
                ]
            }}}}
        ]}),
        "GetAgent" => json!({"agent":{"name":"SDK title"}}),
        _ => panic!("unexpected method {method}"),
    };
    let body = serde_json::to_vec(&response).expect("response JSON");
    write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).expect("response headers");
    socket.write_all(&body).expect("response body");
}
fn until(worker: &mut Worker, predicate: impl Fn(&WorkerEvent) -> bool) -> Vec<WorkerEvent> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut events = Vec::new();
    while Instant::now() < deadline {
        if let Some(event) = worker.poll() {
            let done = predicate(&event);
            events.push(event);
            if done {
                return events;
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
    panic!("timed out; events: {events:?}");
}

#[test]
#[ignore = "uses the installed Cursor SDK and signed-in account for one short sandboxed turn"]
fn live_cursor_sdk_sandboxed_turn() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let config = AgentLaunchConfig {
        program: super::super::program(),
        access_mode: HarnessAccessMode::Auto,
        session_locator_root: Some(project.path().join("state")),
        ..Default::default()
    };
    let (mut worker, _) =
        Worker::start(&config, project.path(), None, None, Some(thread::current()))?;
    worker
        .submit_prompt(
            "live-hi".into(),
            "Hi. Please greet me and say how you can help in one sentence. Do not use tools."
                .into(),
            WorkerSendMode::Prompt,
            Vec::new(),
        )
        .map_err(|error| format!("{error:?}"))?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut delivered = false;
    let mut streamed = String::new();
    let mut text_indices = std::collections::BTreeSet::new();
    let mut fragments = 0;
    let result = 'run: loop {
        while let Some(event) = worker.poll() {
            match event {
                WorkerEvent::Activity(WorkerActivity::InputDelivered { .. }) => delivered = true,
                WorkerEvent::Activity(WorkerActivity::TextDelta {
                    content_index,
                    delta,
                }) => {
                    text_indices.insert(content_index);
                    streamed.push_str(&delta);
                    fragments += 1;
                }
                WorkerEvent::Settled { output } => {
                    if !delivered || output.trim().is_empty() {
                        break 'run Err("Turn settled without delivery and output".into());
                    }
                    if text_indices.len() != 1 || fragments < 2 || streamed != output {
                        break 'run Err(format!(
                            "Invalid text stream: {fragments} fragments, indices {text_indices:?}, streamed {streamed:?}, output {output:?}"
                        ));
                    }
                    eprintln!("Cursor streamed {fragments} fragments into one block: {streamed}");
                    break 'run Ok(());
                }
                WorkerEvent::Failed(error) => break 'run Err(error),
                _ => {}
            }
        }
        if Instant::now() >= deadline {
            break Err("Sandboxed Cursor turn timed out".into());
        }
        thread::park_timeout(Duration::from_millis(100));
    };
    let _ = worker.close();
    result
}

#[test]
fn cancellation_deadline_emits_failure_without_another_wakeup() {
    let fixture = Fixture::new(false, false);
    let mut worker = fixture.worker(None);
    worker.metadata_request = None;
    let (_sender, receiver) = mpsc::sync_channel(1);
    worker.stream = Some(receiver);
    worker.cancelled = Some(Instant::now() - Duration::from_secs(31));
    assert!(
        matches!(worker.poll(), Some(WorkerEvent::Failed(error)) if error == "Cursor SDK cancellation timed out")
    );
}

#[test]
fn buffered_status_frames_do_not_hide_a_terminal_failure() {
    let fixture = Fixture::new(false, false);
    let mut worker = fixture.worker(None);
    worker.metadata_request = None;
    let (sender, receiver) = mpsc::sync_channel(8);
    worker.stream = Some(receiver);
    worker.current = Some(PromptInput {
        submission_id: Some("hi".into()),
        message: "hi".into(),
        mode: WorkerSendMode::Prompt,
        images: Vec::new(),
    });
    let error =
        "Local SDK sandboxing was requested, but sandboxing is not supported in this environment.";
    for frame in [
        json!({"sdkMessage":{"type":"request","message":{"run_id":"run-1"}}}),
        json!({"sdkMessage":{"type":"status","message":{"run_id":"run-1","status":"RUNNING"}}}),
        json!({"sdkMessage":{"type":"status","message":{"run_id":"run-1","status":"ERROR","message":error}}}),
        json!({"result":{"runId":"run-1","status":"RUN_LIFECYCLE_STATUS_ERROR","errorCode":error}}),
        json!({"done":{"runId":"run-1"}}),
    ] {
        sender.send(Ok(Some(frame))).expect("frame");
    }
    sender.send(Ok(None)).expect("end");
    // One wake can cover the whole batch. The runtime stops draining on None.
    assert!(
        matches!(worker.poll(), Some(WorkerEvent::PromptDeliveryUnknown { submission_id, .. }) if submission_id == "hi")
    );
    assert!(matches!(worker.poll(), Some(WorkerEvent::Failed(message)) if message == error));
    assert!(worker.poll().is_none());
}

#[test]
fn missing_api_key_reaches_sdk_for_new_and_resumed_sessions() {
    for resume in [None, Some("sdk-agent")] {
        let fixture = Fixture::new(false, false);
        let (mut worker, _) = Worker::from_bridge(
            Bridge::fixture(fixture.client.clone(), None),
            &AgentLaunchConfig::default(),
            fixture.project.as_path(),
            resume,
            None,
            None,
        )
        .expect("SDK accepts requests without an explicit key");
        let requests = fixture.requests.lock().expect("requests");
        for method in [
            "ListModels",
            if resume.is_some() {
                "ResumeAgent"
            } else {
                "CreateAgent"
            },
        ] {
            let body = &requests.iter().find(|(m, _)| m == method).expect(method).1;
            assert!(body["options"].get("apiKey").is_none());
        }
        drop(requests);
        worker.close().expect("close");
    }
}

#[test]
fn sdk_turns_stream_once_and_queued_followup_uses_same_agent() {
    let fixture = Fixture::new(false, false);
    let mut worker = fixture.worker(None);
    worker
        .submit_prompt("one".into(), "hello".into(), WorkerSendMode::Prompt, vec![])
        .expect("first prompt");
    worker
        .submit_prompt("two".into(), "again".into(), WorkerSendMode::Queue, vec![])
        .expect("followup");
    let first = until(&mut worker, |e| matches!(e, WorkerEvent::Settled { .. }));
    let text: Vec<_> = first
        .iter()
        .filter_map(|event| match event {
            WorkerEvent::Activity(WorkerActivity::TextDelta {
                content_index,
                delta,
            }) => Some((*content_index, delta.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(text, vec![(0, "hel"), (0, "lo")]);
    assert_eq!(first.iter().filter(|e| matches!(e,WorkerEvent::Activity(WorkerActivity::InputDelivered {submission_id:Some(id),..}) if id == "one")).count(),1);
    until(&mut worker, |e| matches!(e, WorkerEvent::Settled { .. }));
    let requests = fixture.requests.lock().expect("requests");
    let sends: Vec<_> = requests.iter().filter(|(m, _)| m == "Send").collect();
    assert_eq!(sends.len(), 2);
    assert!(
        sends
            .iter()
            .all(|(_, body)| body["options"]["enableDeltas"] == true)
    );
    assert!(sends.iter().all(|(_, body)| body["agentId"] == "sdk-agent"));
    assert_eq!(sends[1].1["message"]["text"], "again");
    let create = &requests
        .iter()
        .find(|(m, _)| m == "CreateAgent")
        .expect("create")
        .1;
    assert_eq!(
        create["options"]["local"]["sandboxOptions"]["enabled"],
        true
    );
    drop(requests);
    worker.close().expect("close");
}

#[test]
fn cancellation_before_run_id_waits_for_native_terminal_and_cancels_queue() {
    let fixture = Fixture::new(true, false);
    let mut worker = fixture.worker(Some("sdk-agent"));
    worker
        .submit_prompt("one".into(), "wait".into(), WorkerSendMode::Prompt, vec![])
        .expect("prompt");
    worker
        .submit_prompt("two".into(), "queued".into(), WorkerSendMode::Queue, vec![])
        .expect("queue");
    worker.abort().expect("early cancellation");
    let events = until(&mut worker, |e| matches!(e, WorkerEvent::Settled { .. }));
    assert!(events.iter().any(
        |e| matches!(e,WorkerEvent::PromptCancelled {submission_id} if submission_id == "two")
    ));
    assert!(!events.iter().any(|e| matches!(
        e,
        WorkerEvent::Activity(WorkerActivity::InputDelivered { .. })
    )));
    assert!(
        fixture
            .requests
            .lock()
            .expect("requests")
            .iter()
            .any(|(m, v)| m == "CancelRun" && v["runId"] == "run-0")
    );
    worker.close().expect("close");
}

#[test]
fn truncated_stream_fails_and_never_reports_settlement() {
    let fixture = Fixture::new(false, true);
    let mut worker = fixture.worker(None);
    worker
        .send("hello".into(), WorkerSendMode::Prompt)
        .expect("prompt");
    let events = until(&mut worker, |e| matches!(e, WorkerEvent::Failed(_)));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, WorkerEvent::Settled { .. }))
    );
    assert!(worker.has_exited());
}

#[test]
fn resume_restores_model_parameters_and_history() {
    let fixture = Fixture::new(false, false);
    let mut worker = fixture.worker(Some("sdk-agent"));
    assert_eq!(worker.model.tier().as_deref(), Some("priority"));
    let history = super::super::history::load(&worker.bridge, "sdk-agent", Path::new("/fixture"))
        .expect("history");
    assert_eq!(history.messages.len(), 2);
    assert_eq!(history.messages[0]["role"], "user");
    assert_eq!(history.messages[0]["content"][0]["text"], "Earlier prompt");
    assert_eq!(history.messages[1]["role"], "assistant");
    assert_eq!(
        history.messages[1]["content"][0]["thinking"],
        "Considering the prompt"
    );
    assert_eq!(history.messages[1]["content"][1]["text"], "Earlier reply");
    worker.close().expect("close");
}

#[test]
fn peer_reports_keep_their_identity_on_delivery() {
    let fixture = Fixture::new(false, false);
    let mut worker = fixture.worker(None);
    let message = crate::PeerMessage {
        from: "reviewer".into(),
        message: "Found an issue".into(),
    };
    worker
        .send_peer_message(&message, WorkerSendMode::Prompt)
        .expect("peer message");
    let events = until(&mut worker, |e| matches!(e, WorkerEvent::Settled { .. }));
    assert_eq!(events.iter().filter(|e|matches!(e,WorkerEvent::Activity(WorkerActivity::PeerInputDelivered {message:delivered}) if delivered == &message)).count(),1);
    assert!(!events.iter().any(|e| matches!(
        e,
        WorkerEvent::Activity(WorkerActivity::InputDelivered { .. })
    )));
    worker.close().expect("close");
}

#[test]
fn steering_acknowledgements_preserve_repeated_inputs_and_fallback_ownership() {
    let fixture = Fixture::new(true, false);
    let mut worker = fixture.worker(None);
    worker
        .submit_prompt(
            "first".into(),
            "work".into(),
            WorkerSendMode::Prompt,
            vec![],
        )
        .unwrap();
    for id in ["steer-1", "steer-2", "fallback"] {
        let text = if id == "fallback" {
            "fallback"
        } else {
            "same text"
        };
        assert!(
            !worker
                .submit_prompt(id.into(), text.into(), WorkerSendMode::Steer, vec![])
                .unwrap()
        );
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut acks = Vec::new();
    while acks.len() < 3 && Instant::now() < deadline {
        if let Some(ack) = worker.poll_prompt_ack() {
            acks.push(ack);
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(acks.len(), 3);
    assert_eq!(acks.iter().filter(|(_, result)| result.is_ok()).count(), 2);
    assert!(acks.iter().any(|(id, result)| {
        id == "fallback"
            && result
                .as_ref()
                .is_err_and(|error| error.message == STEER_FOLLOW_UP)
    }));
    let events: Vec<_> = std::iter::from_fn(|| worker.poll()).collect();
    for id in ["first", "steer-1", "steer-2"] {
        assert_eq!(events.iter().filter(|event| matches!(event, WorkerEvent::Activity(WorkerActivity::InputDelivered { submission_id: Some(delivered), .. }) if delivered == id)).count(), 1);
    }
    assert!(worker.queue.is_empty(), "shared queue owns fallback");
    assert_eq!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(method, _)| method == "Send")
            .count(),
        1
    );
    worker.close().unwrap();
}

#[test]
fn unknown_steering_outcome_never_becomes_a_retry() {
    let fixture = Fixture::new(false, false);
    let mut worker = fixture.worker(None);
    let (sender, receiver) = mpsc::channel();
    worker.steering.push((
        PromptInput {
            submission_id: Some("steer".into()),
            message: "redirect".into(),
            mode: WorkerSendMode::Steer,
            images: vec![],
        },
        receiver,
    ));
    sender
        .send(Err("connection lost after write".into()))
        .unwrap();
    assert!(worker.poll_prompt_ack().is_none());
    assert!(
        matches!(worker.poll(), Some(WorkerEvent::PromptDeliveryUnknown { submission_id, .. }) if submission_id == "steer")
    );
    assert!(worker.queue.is_empty());
    worker.close().unwrap();
}

#[test]
#[ignore = "uses the signed-in Cursor SDK for a sandboxed tool call and native steering"]
fn live_cursor_sdk_native_steering() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let config = AgentLaunchConfig {
        program: super::super::program(),
        access_mode: HarnessAccessMode::Auto,
        session_locator_root: Some(project.path().join("state")),
        ..Default::default()
    };
    let (mut worker, _) =
        Worker::start(&config, project.path(), None, None, Some(thread::current()))?;
    worker
        .submit_prompt(
            "work".into(),
            "Run the shell command `sleep 5`, then say ready. Do not edit files.".into(),
            WorkerSendMode::Prompt,
            vec![],
        )
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut steered = false;
    let mut delivered = 0;
    let mut acknowledged = false;
    let result = 'run: loop {
        while let Some((id, result)) = worker.poll_prompt_ack() {
            if id == "steer" {
                if let Err(error) = result {
                    break 'run Err(error.to_string());
                }
                acknowledged = true;
            }
        }
        while let Some(event) = worker.poll() {
            match event {
                WorkerEvent::Activity(WorkerActivity::ToolStarted { .. }) if !steered => {
                    worker
                        .submit_prompt(
                            "steer".into(),
                            "Change the final answer to exactly CURSOR_STEER_CONFIRMED.".into(),
                            WorkerSendMode::Steer,
                            vec![],
                        )
                        .map_err(|error| error.to_string())?;
                    steered = true;
                }
                WorkerEvent::Activity(WorkerActivity::InputDelivered {
                    submission_id: Some(id),
                    ..
                }) if id == "steer" => delivered += 1,
                WorkerEvent::Settled { output } => {
                    while let Some((id, result)) = worker.poll_prompt_ack() {
                        if id == "steer" {
                            acknowledged = result.is_ok();
                        }
                    }
                    break 'run if acknowledged
                        && delivered == 1
                        && output.contains("CURSOR_STEER_CONFIRMED")
                    {
                        Ok(())
                    } else {
                        Err(format!(
                            "Steering not confirmed: ack={acknowledged}, delivered={delivered}, output={output:?}"
                        ))
                    };
                }
                WorkerEvent::Failed(error) => break 'run Err(error),
                _ => {}
            }
        }
        if Instant::now() >= deadline {
            break Err("Native steering test timed out".into());
        }
        thread::park_timeout(Duration::from_millis(100));
    };
    let _ = worker.close();
    result
}

#[test]
fn new_and_resumed_sessions_reuse_the_central_catalog() {
    let fixture = Fixture::new(false, false);
    let legacy = serde_json::from_value(json!({"models":[{
        "id":"test-model", "name":"Test", "provider":Backend::Cursor
    }],"efforts":[]}))
    .unwrap();
    crate::seed_configuration_catalog(
        &AgentLaunchConfig::default(),
        Backend::Cursor,
        &fixture.project,
        &legacy,
    );
    let mut first = fixture.worker(None);
    first.close().unwrap();
    let mut resumed = fixture.worker(Some("sdk-agent"));
    assert_eq!(resumed.model.tier().as_deref(), Some("priority"));
    resumed.close().unwrap();
    let requests = fixture.requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|(method, _)| method == "ListModels")
            .count(),
        1
    );
}

#[test]
fn persisted_central_catalog_skips_sdk_model_lookup() {
    let fixture = Fixture::new(false, false);
    let raw = vec![json!({"id":"test-model","parameters":[
        {"id":"fast","values":[{"value":"false"},{"value":"true"}]}
    ]})];
    let models = configuration::models(&raw);
    let catalog =
        super::super::super::configuration_catalog(configuration::metadata(&models, &models[0]))
            .unwrap();
    let persisted = serde_json::to_vec(&catalog).unwrap();
    let restored = serde_json::from_slice(&persisted).unwrap();
    crate::seed_configuration_catalog(
        &AgentLaunchConfig::default(),
        Backend::Cursor,
        &fixture.project,
        &restored,
    );
    let mut fresh = fixture.worker(None);
    fresh.close().unwrap();
    let mut resumed = fixture.worker(Some("sdk-agent"));
    assert_eq!(resumed.model.tier().as_deref(), Some("priority"));
    resumed.close().unwrap();
    let requests = fixture.requests.lock().unwrap();
    assert!(!requests.iter().any(|(method, _)| method == "ListModels"));
    let (_, create) = requests
        .iter()
        .find(|(method, _)| method == "CreateAgent")
        .unwrap();
    assert_eq!(
        create["options"]["model"],
        json!({"id":"test-model","params":[{"id":"fast","value":"false"}]})
    );
}
