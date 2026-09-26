use super::*;
use crate::Backend;
use crate::HarnessAccessMode;
use std::{error::Error, fs};
use tempfile::tempdir;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

struct DisabledMcp {
    _guard: crate::builtin_mcp::McpDisabledForTest,
}

impl DisabledMcp {
    fn new() -> Self {
        Self {
            _guard: crate::builtin_mcp::McpDisabledForTest::new(),
        }
    }
}

fn pi_test_command(script: &Path, arguments: Vec<String>) -> AgentLaunchConfig {
    let mut command = AgentLaunchConfig::test_script(script, arguments);
    command.access_mode = HarnessAccessMode::Full;
    command
}

fn fake(case: &str) -> TestResult<(tempfile::TempDir, AgentLaunchConfig)> {
    let temp = tempdir()?;
    let script = temp.path().join("fake.sh");
    fs::write(
        &script,
        include_str!("../../../../../tests/fixtures/fake-pi.sh"),
    )?;
    Ok((temp, pi_test_command(&script, vec![case.into()])))
}

fn queue_rpc_fixture(project: &Path) -> TestResult<AgentLaunchConfig> {
    queue_rpc_fixture_case(project, "normal")
}

fn queue_rpc_fixture_case(project: &Path, case: &str) -> TestResult<AgentLaunchConfig> {
    let script = project.join("queue-rpc.sh");
    fs::write(&script, include_str!("test_queue_rpc.sh"))?;
    Ok(pi_test_command(&script, vec![case.into()]))
}

#[test]
fn compact_slash_command_preserves_prompt_correlation_and_settles() -> TestResult {
    use crate::SessionTransport;
    for case in ["normal", "compaction-fails"] {
        let project = tempdir()?;
        let command = queue_rpc_fixture_case(project.path(), case)?;
        let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
        let id = rpc.send_request(SessionCommand::Prompt {
            mode: crate::extensions::PromptMode::Normal,
            message: "/compact keep decisions".into(),
            images: vec![],
        })?;
        let response = loop {
            let item = rpc.incoming.recv_timeout(Duration::from_secs(5))?;
            match rpc.route(item) {
                SessionEvent::Response(response) if response.id.as_deref() == Some(&id) => {
                    break response;
                }
                other => rpc.queued.push_back(other),
            }
        };
        assert_eq!(
            response.operation(),
            crate::SessionOperation::Prompt(crate::extensions::PromptMode::Normal)
        );
        assert_eq!(response.result.is_ok(), case == "normal");
        assert_eq!(rpc.activity, WorkerActivityState::Idle);
        assert!(rpc.pending_prompt_modes.is_empty());
        assert!(rpc.queued.iter().any(|event| matches!(event, SessionEvent::Activity(event) if event.value()["type"] == "agent_settled")));
        let requests = fs::read_to_string(project.path().join("fixture-rpc-lines"))?;
        let compact: Value = requests
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|row| row["type"] == "compact")
            .ok_or("no compact RPC")?;
        assert_eq!(compact["customInstructions"], "keep decisions");
        assert!(!requests.contains("\"message\":\"/compact"));
        rpc.set_activity(WorkerActivityState::Working);
        assert!(
            rpc.send_request(SessionCommand::Prompt {
                mode: crate::extensions::PromptMode::Normal,
                message: "/compact".into(),
                images: vec![]
            })
            .is_err()
        );
        rpc.close()?;
    }
    Ok(())
}

fn installed_pi_fixture(project: &Path) -> TestResult<AgentLaunchConfig> {
    let pi = resolve_agent_program(
        Path::new("pi"),
        project,
        std::env::var_os("PATH").as_deref(),
    )
    .map_err(|error| format!("installed Pi prerequisite: {error}"))?;
    let wrapper = project.join("installed-pi.sh");
    fs::write(&wrapper, include_str!("test_installed_pi.sh"))?;
    let extension =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapter/pi/test_fixture_provider.js");
    Ok(pi_test_command(
        &wrapper,
        vec![
            pi.to_string_lossy().into_owned(),
            "--extension".into(),
            extension.to_string_lossy().into_owned(),
            "--provider".into(),
            "farcaster-fixture".into(),
            "--model".into(),
            "fixture".into(),
        ],
    ))
}

fn assert_installed_model(rpc: &mut PiRpcProcess, expected: &str) -> TestResult {
    let response = rpc.request_and_wait(SessionCommand::LoadState)?;
    let crate::SessionResponsePayload::LoadState(state) = response.result? else {
        return Err("expected Pi state".into());
    };
    let model = state.model.ok_or("missing selected model")?;
    assert_eq!(
        (model.provider.as_str(), model.id.as_str()),
        ("farcaster-fixture", expected)
    );
    Ok(())
}

fn assert_installed_default(project: &Path, expected: &str) -> TestResult {
    let settings: Value =
        serde_json::from_str(&fs::read_to_string(project.join("pi-agent/settings.json"))?)?;
    assert_eq!(settings["defaultProvider"], "farcaster-fixture");
    assert_eq!(
        settings["defaultModel"], expected,
        "automatic launch changed Pi's saved selection"
    );
    Ok(())
}

fn installed_default_thinking(project: &Path) -> TestResult<Value> {
    let settings: Value =
        serde_json::from_str(&fs::read_to_string(project.join("pi-agent/settings.json"))?)?;
    Ok(settings["defaultThinkingLevel"].clone())
}

#[test]
#[ignore = "requires installed Pi; real RPC with isolated settings and local provider, no network"]
fn installed_pi_child_model_does_not_replace_the_users_selected_default() -> TestResult {
    use crate::{WorkerContext, WorkerEvent, WorkerLaunch, WorkerSessionFactory};
    let _mcp = DisabledMcp::new();
    let project = tempdir()?;
    let command = installed_pi_fixture(project.path())?;
    let mut parent = PiRpcProcess::spawn(&command, project.path(), None)?;
    parent.request_and_wait(SessionCommand::SelectModel {
        provider: "farcaster-fixture".into(),
        model_id: "fixture".into(),
    })?;
    let factory = super::super::worker::PiWorkerFactory::new(command.clone());
    let original_thinking = installed_default_thinking(project.path())?;
    let mut context = WorkerContext::Fresh;
    let mut saved_locator = None;
    for index in 0..2 {
        let mut child = factory.create(WorkerLaunch {
            slot: None,
            worker_id: format!("model-child-{index}"),
            worker_name: format!("model-child-{index}"),
            project: project.path().to_path_buf(),
            parent_session: "parent".into(),
            parent_worker_id: None,
            context,
            provider: Some("farcaster-fixture".into()),
            model: Some("fixture-child".into()),
            effort: Some("high".into()),
            service_tier: None,
            access_mode: HarnessAccessMode::Full,
            app_proxy: None,
            ephemeral: false,
        })?;
        // Check the real settings file immediately, not just launch arguments.
        assert_installed_default(project.path(), "fixture")?;
        assert_eq!(
            installed_default_thinking(project.path())?,
            original_thinking
        );
        child.send(format!("child turn {index}"), WorkerSendMode::Prompt)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut locator = None;
        let mut settled = false;
        while Instant::now() < deadline && (!settled || locator.is_none()) {
            match child.poll() {
                Some(WorkerEvent::SessionChanged { locator: path }) => locator = Some(path),
                Some(WorkerEvent::Settled { output }) => {
                    assert!(output.contains(&format!("child turn {index}")));
                    settled = true;
                }
                Some(WorkerEvent::Failed(error) | WorkerEvent::RequestFailed { error, .. }) => {
                    return Err(error.into());
                }
                _ => thread::sleep(Duration::from_millis(10)),
            }
        }
        child.close()?;
        assert!(settled, "real Pi child did not complete");
        let locator = locator.ok_or("child omitted locator")?;
        if let Some(previous) = &saved_locator {
            assert_eq!(&locator, previous);
        }
        let history = fs::read_to_string(&locator)?;
        let messages = history
            .lines()
            .map(serde_json::from_str::<Value>)
            .collect::<Result<Vec<_>, _>>()?;
        assert!(
            messages
                .iter()
                .any(|entry| entry["message"]["role"] == "assistant"
                    && entry["message"]["model"] == "fixture-child")
        );
        saved_locator = Some(locator.clone());
        context = WorkerContext::Resume {
            session_locator: locator,
        };
        assert_installed_default(project.path(), "fixture")?;
        assert_installed_model(&mut parent, "fixture")?;
    }
    parent.terminate()?;
    let mut default_command = command;
    default_command
        .prefix_args
        .truncate(default_command.prefix_args.len() - 4);
    let mut next = PiRpcProcess::spawn(&default_command, project.path(), None)?;
    assert_installed_model(&mut next, "fixture")?;
    next.terminate()?;
    Ok(())
}

#[test]
#[ignore = "requires installed Pi; real RPC with isolated settings and local provider, no network"]
fn installed_pi_abort_preserves_its_model_without_overwriting_another_sessions_selection()
-> TestResult {
    let _mcp = DisabledMcp::new();
    let project = tempdir()?;
    let command = installed_pi_fixture(project.path())?;
    let mut first = PiRpcProcess::spawn(&command, project.path(), None)?;
    first.request_and_wait(SessionCommand::SelectModel {
        provider: "farcaster-fixture".into(),
        model_id: "fixture-child".into(),
    })?;
    first.request_and_wait(SessionCommand::SelectReasoning {
        level: "high".into(),
    })?;
    prompt(
        &mut first,
        crate::extensions::PromptMode::Normal,
        "persist selected model",
    )?;
    wait_for_activity(&mut first, crate::SessionActivityKind::AgentSettled)?;
    let path = first
        .session_locator
        .clone()
        .ok_or("missing first session")?;
    let mut second = PiRpcProcess::spawn(&command, project.path(), None)?;
    second.request_and_wait(SessionCommand::SelectModel {
        provider: "farcaster-fixture".into(),
        model_id: "fixture-other".into(),
    })?;
    second.request_and_wait(SessionCommand::SelectReasoning {
        level: "low".into(),
    })?;
    first.send_request(SessionCommand::Abort)?;
    assert_installed_model(&mut first, "fixture-child")?;
    assert_eq!(first.selected_reasoning.as_deref(), Some("high"));
    assert_eq!(first.session_locator.as_ref(), Some(&path));
    assert_installed_model(&mut second, "fixture-other")?;
    assert_installed_default(project.path(), "fixture-other")?;
    assert_eq!(installed_default_thinking(project.path())?, "low");
    first.terminate()?;
    second.terminate()?;
    let mut default_command = command;
    default_command
        .prefix_args
        .truncate(default_command.prefix_args.len() - 4);
    let mut next = PiRpcProcess::spawn(&default_command, project.path(), None)?;
    assert_installed_model(&mut next, "fixture-other")?;
    next.terminate()?;
    Ok(())
}

fn wait_for_activity(rpc: &mut PiRpcProcess, expected: crate::SessionActivityKind) -> TestResult {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        match rpc.try_next() {
            Some(SessionEvent::Activity(activity)) if *activity.kind() == expected => return Ok(()),
            Some(SessionEvent::Failure(error)) => return Err(error.into()),
            _ => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Err(format!("Pi did not emit {expected:?}").into())
}

fn prompt(
    rpc: &mut PiRpcProcess,
    mode: crate::extensions::PromptMode,
    message: &str,
) -> TestResult<String> {
    Ok(rpc.send_request(SessionCommand::Prompt {
        mode,
        message: message.into(),
        images: Vec::new(),
    })?)
}

fn wait_for_response(rpc: &mut PiRpcProcess, expected_id: &str) -> TestResult {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        match rpc.try_next() {
            Some(SessionEvent::Response(response))
                if response.id.as_deref() == Some(expected_id) =>
            {
                response.result.map_err(|error| error.to_string())?;
                return Ok(());
            }
            Some(SessionEvent::Failure(error)) => return Err(error.into()),
            _ => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Err(format!("Pi did not acknowledge {expected_id}").into())
}

fn wait_for_established_installed_request(
    rpc: &mut PiRpcProcess,
    project: &Path,
    message: &str,
) -> TestResult {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let provider_received = fs::read_to_string(project.join("fixture-requests"))
            .is_ok_and(|requests| requests.contains(message));
        let session_persisted = rpc.session_locator.as_ref().is_some_and(|session| {
            fs::read_to_string(session).is_ok_and(|history| history.contains(message))
        });
        if provider_received && session_persisted {
            return Ok(());
        }
        if let Some(SessionEvent::Failure(error)) = rpc.try_next() {
            return Err(error.into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err(format!("installed Pi did not establish request: {message}").into())
}

#[test]
fn abort_stops_active_pi_work_and_discards_steering_and_follow_up() -> TestResult {
    let project = tempdir()?;
    let command = queue_rpc_fixture(project.path())?;
    let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
    prompt(
        &mut rpc,
        crate::extensions::PromptMode::Normal,
        "hold active",
    )?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentStarted)?;
    let steering = prompt(
        &mut rpc,
        crate::extensions::PromptMode::Steer,
        "discard steering",
    )?;
    let follow_up = prompt(
        &mut rpc,
        crate::extensions::PromptMode::FollowUp,
        "discard follow-up",
    )?;
    wait_for_response(&mut rpc, &steering)?;
    wait_for_response(&mut rpc, &follow_up)?;
    rpc.send_request(SessionCommand::Abort)?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentSettled)?;

    prompt(
        &mut rpc,
        crate::extensions::PromptMode::Normal,
        "submit again",
    )?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentSettled)?;
    let requests = fs::read_to_string(project.path().join("fixture-requests"))?;
    assert_eq!(
        requests.lines().collect::<Vec<_>>(),
        ["hold active", "submit again"]
    );
    let sessions = fs::read_to_string(project.path().join("fixture-sessions"))?;
    assert_eq!(
        sessions
            .lines()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        1
    );
    let pids = fs::read_to_string(project.path().join("fixture-pids"))?;
    assert_eq!(
        pids.lines().collect::<std::collections::HashSet<_>>().len(),
        2
    );
    rpc.terminate()?;
    Ok(())
}

#[test]
fn apply_steering_runs_all_queued_pi_inputs() -> TestResult {
    let project = tempdir()?;
    let command = queue_rpc_fixture(project.path())?;
    let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
    prompt(
        &mut rpc,
        crate::extensions::PromptMode::Normal,
        "hold before steering",
    )?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentStarted)?;
    let queued = [
        prompt(
            &mut rpc,
            crate::extensions::PromptMode::FollowUp,
            "equal queued",
        )?,
        prompt(
            &mut rpc,
            crate::extensions::PromptMode::FollowUp,
            "second follow-up",
        )?,
        prompt(
            &mut rpc,
            crate::extensions::PromptMode::Steer,
            "equal queued",
        )?,
        prompt(
            &mut rpc,
            crate::extensions::PromptMode::Steer,
            "second steer",
        )?,
    ];
    for id in queued {
        wait_for_response(&mut rpc, &id)?;
    }
    assert_eq!(
        fs::read_to_string(project.path().join("fixture-admissions"))?,
        concat!(
            "follow_up:equal queued\n",
            "follow_up:second follow-up\n",
            "steer:equal queued\n",
            "steer:second steer\n",
        )
    );
    let apply = rpc.send_request(SessionCommand::ApplySteering)?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut apply_acknowledged = false;
    let mut settled = false;
    while Instant::now() < deadline && !(apply_acknowledged && settled) {
        match rpc.try_next() {
            Some(SessionEvent::Response(response)) if response.id.as_deref() == Some(&apply) => {
                assert_eq!(response.operation(), crate::SessionOperation::ApplySteering);
                response.result?;
                apply_acknowledged = true;
            }
            Some(SessionEvent::Activity(activity))
                if activity.kind() == &crate::SessionActivityKind::AgentSettled =>
            {
                settled = true;
            }
            Some(SessionEvent::Failure(error)) => return Err(error.into()),
            _ => {}
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(apply_acknowledged && settled);

    let requests = fs::read_to_string(project.path().join("fixture-requests"))?;
    assert_eq!(
        requests.lines().collect::<Vec<_>>(),
        [
            "hold before steering",
            "equal queued",
            "second steer",
            "equal queued",
            "second follow-up",
        ]
    );
    rpc.terminate()?;
    Ok(())
}

#[test]
fn configuring_pi_queues_sets_steering_and_follow_up_to_all() -> TestResult {
    let project = tempdir()?;
    let command = queue_rpc_fixture(project.path())?;
    let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
    rpc.request_and_wait(SessionCommand::ConfigureSteering)?;
    assert_eq!(
        fs::read_to_string(project.path().join("fixture-steering-configurations"))?,
        "steering:all\nfollow_up:all\n"
    );
    rpc.terminate()?;
    Ok(())
}

#[test]
fn transformed_native_user_event_remains_visible_without_false_correlation() -> TestResult {
    let project = tempdir()?;
    let command = queue_rpc_fixture(project.path())?;
    let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
    assert!(crate::SessionTransport::tracks_prompt_delivery(
        &rpc,
        crate::extensions::PromptMode::Normal,
    ));
    let id = rpc.send_request(SessionCommand::Prompt {
        mode: crate::extensions::PromptMode::Normal,
        message: "event-first-transform image".into(),
        images: vec![crate::extensions::PromptImage::new(
            "aGVsbG8=".into(),
            "image/png".into(),
        )],
    })?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut acknowledged = false;
    let mut raw_user_events = Vec::new();
    while Instant::now() < deadline && !(acknowledged && raw_user_events.len() == 2) {
        match rpc.try_next() {
            Some(SessionEvent::Response(response)) if response.id.as_deref() == Some(&id) => {
                response.result?;
                acknowledged = true;
            }
            Some(SessionEvent::Activity(activity))
                if matches!(
                    activity.value().get("type").and_then(Value::as_str),
                    Some("message_start" | "message_end")
                ) && activity.value()["message"]["role"] == "user" =>
            {
                raw_user_events.push(activity.value()["message"].clone());
            }
            Some(SessionEvent::Failure(error)) => return Err(error.into()),
            _ => {}
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(acknowledged);
    assert_eq!(raw_user_events.len(), 2);
    for message in raw_user_events {
        assert_eq!(message["content"][0]["text"], "extension transformed");
        assert_eq!(
            message["content"][1],
            serde_json::json!({
                "type": "image",
                "data": "dHJhbnNmb3JtZWQ=",
                "mimeType": "image/webp",
            })
        );
    }
    rpc.terminate()?;
    Ok(())
}

#[test]
fn main_prompt_delivery_survives_both_rpc_event_orders() -> TestResult {
    for text in ["response-first", "event-first"] {
        let project = tempdir()?;
        let command = queue_rpc_fixture(project.path())?;
        let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
        let id = rpc.send_request(SessionCommand::Prompt {
            mode: crate::extensions::PromptMode::Normal,
            message: text.into(),
            images: vec![],
        })?;
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut receipts = 0;
        let mut replied = false;
        let mut settled = false;
        while Instant::now() < deadline && !(replied && settled) {
            match rpc.try_next() {
                Some(SessionEvent::Response(response)) if response.id.as_deref() == Some(&id) => {
                    assert!(response.result.is_ok());
                    replied = true;
                }
                Some(SessionEvent::Activity(event)) => {
                    let value = event.value();
                    if value["type"] == "prompt_delivery" {
                        assert_eq!(value["submissionId"], id);
                        assert_eq!(value["status"], "delivered");
                        receipts += 1;
                    }
                    if matches!(
                        value["type"].as_str(),
                        Some("message_start" | "message_end")
                    ) {
                        assert_ne!(
                            value["message"]["role"], "user",
                            "raw user echo duplicated receipt"
                        );
                    }
                    settled |= value["type"] == "agent_settled";
                }
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        rpc.terminate()?;
        assert!(replied && settled, "{text}");
        assert_eq!(receipts, 1, "{text}");
    }
    Ok(())
}

#[test]
fn resumed_pi_history_loads_one_persisted_prompt() -> TestResult {
    let project = tempdir()?;
    let command = queue_rpc_fixture_case(project.path(), "history")?;
    let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
    let id = prompt(
        &mut rpc,
        crate::extensions::PromptMode::Normal,
        "history prompt",
    )?;
    wait_for_response(&mut rpc, &id)?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentSettled)?;
    let session = rpc
        .session_locator
        .clone()
        .ok_or("missing session locator")?;
    rpc.terminate()?;

    let mut resumed = PiRpcProcess::spawn(&command, project.path(), Some(&session))?;
    let response = resumed.request_and_wait(SessionCommand::LoadHistory)?;
    let crate::SessionResponsePayload::LoadHistory(crate::SessionHistory::Replace {
        messages: history,
        ..
    }) = response.result?
    else {
        return Err("expected replacement history".into());
    };
    let user = history
        .iter()
        .filter(|message| message["role"] == "user")
        .count();
    assert_eq!(user, 1, "{history:?}");
    resumed.terminate()?;
    Ok(())
}

#[test]
fn abort_marks_only_unacknowledged_dispatched_prompt_unknown() -> TestResult {
    let project = tempdir()?;
    let command = queue_rpc_fixture(project.path())?;
    let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
    let id = prompt(
        &mut rpc,
        crate::extensions::PromptMode::Normal,
        "unconfirmed dispatch",
    )?;
    rpc.send_request(SessionCommand::Abort)?;
    let mut response = None;
    while let Some(event) = rpc.try_next() {
        if let SessionEvent::Response(candidate) = event
            && candidate.id.as_deref() == Some(&id)
        {
            response = Some(candidate);
        }
    }
    let error = response
        .ok_or("missing delivery-unknown response")?
        .result
        .expect_err("delivery must be unknown");
    assert_eq!(error.kind, crate::SessionResponseErrorKind::DeliveryUnknown);
    rpc.terminate()?;
    Ok(())
}

#[test]
fn second_abort_cancels_an_in_progress_apply_handoff() -> TestResult {
    let project = tempdir()?;
    let command = queue_rpc_fixture_case(project.path(), "slow-handoff")?;
    let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
    prompt(
        &mut rpc,
        crate::extensions::PromptMode::Normal,
        "hold handoff",
    )?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentStarted)?;
    let steering = prompt(
        &mut rpc,
        crate::extensions::PromptMode::Steer,
        "cancelled steering",
    )?;
    let follow_up = prompt(
        &mut rpc,
        crate::extensions::PromptMode::FollowUp,
        "cancelled follow-up",
    )?;
    wait_for_response(&mut rpc, &steering)?;
    wait_for_response(&mut rpc, &follow_up)?;
    let apply = rpc.send_request(SessionCommand::ApplySteering)?;
    rpc.send_request(SessionCommand::Abort)?;

    let mut cancelled_apply = false;
    while let Some(event) = rpc.try_next() {
        if let SessionEvent::Response(response) = event
            && response.id.as_deref() == Some(&apply)
        {
            assert_eq!(response.operation(), crate::SessionOperation::ApplySteering);
            assert!(response.result.is_err());
            cancelled_apply = true;
        }
    }
    assert!(cancelled_apply);
    prompt(
        &mut rpc,
        crate::extensions::PromptMode::Normal,
        "after cancelled handoff",
    )?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentSettled)?;
    let requests = fs::read_to_string(project.path().join("fixture-requests"))?;
    assert_eq!(
        requests.lines().collect::<Vec<_>>(),
        ["hold handoff", "after cancelled handoff"]
    );
    assert_eq!(
        fs::read_to_string(project.path().join("fixture-sessions"))?
            .lines()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        1
    );
    rpc.terminate()?;
    Ok(())
}

#[test]
fn abort_preserves_only_already_routed_prompt_acknowledgements() -> TestResult {
    let project = tempdir()?;
    let command = queue_rpc_fixture(project.path())?;
    let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
    let steering = prompt(&mut rpc, crate::extensions::PromptMode::Steer, "queued ack")?;
    rpc.send_request(SessionCommand::LoadHistory)?;
    rpc.request_and_wait(SessionCommand::LoadState)?;
    rpc.send_request(SessionCommand::Abort)?;

    let mut operations = Vec::new();
    while let Some(event) = rpc.try_next() {
        if let SessionEvent::Response(response) = event {
            let operation = response.operation();
            operations.push((response.id, operation));
        }
    }
    assert!(operations.iter().any(|(id, operation)| {
        id.as_deref() == Some(&steering)
            && *operation == crate::SessionOperation::Prompt(crate::extensions::PromptMode::Steer)
    }));
    assert!(
        !operations
            .iter()
            .any(|(_, operation)| { *operation == crate::SessionOperation::LoadHistory })
    );
    rpc.terminate()?;
    Ok(())
}

#[test]
fn abort_restores_pi_model_reasoning_and_steering_configuration() -> TestResult {
    let project = tempdir()?;
    let command = queue_rpc_fixture_case(project.path(), "replacement-defaults")?;
    let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
    rpc.request_and_wait(SessionCommand::ConfigureSteering)?;
    rpc.request_and_wait(SessionCommand::SelectModel {
        provider: "fixture".into(),
        model_id: "fixture-model".into(),
    })?;
    rpc.request_and_wait(SessionCommand::SelectReasoning {
        level: "high".into(),
    })?;
    assert!(
        rpc.request_and_wait(SessionCommand::SelectModel {
            provider: "fixture".into(),
            model_id: "rejected-model".into(),
        })
        .is_err()
    );
    assert!(
        rpc.request_and_wait(SessionCommand::SelectReasoning {
            level: "rejected".into(),
        })
        .is_err()
    );
    prompt(
        &mut rpc,
        crate::extensions::PromptMode::Normal,
        "hold settings",
    )?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentStarted)?;
    rpc.send_request(SessionCommand::Abort)?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentSettled)?;
    let response = rpc.request_and_wait(SessionCommand::LoadState)?;
    let crate::SessionResponsePayload::LoadState(state) = response.result? else {
        return Err("expected Pi state".into());
    };
    assert_eq!(
        state.model.as_ref().map(|model| model.id.as_str()),
        Some("fixture-model")
    );
    assert_eq!(state.thinking_level.as_deref(), Some("high"));
    assert_eq!(
        fs::read_to_string(project.path().join("fixture-steering-configurations"))?
            .lines()
            .count(),
        4
    );
    assert_eq!(
        fs::read_to_string(project.path().join("session.jsonl"))?,
        "hold settings\n"
    );
    rpc.terminate()?;
    Ok(())
}

#[test]
fn abort_without_a_session_locator_still_stops_and_starts_fresh() -> TestResult {
    let project = tempdir()?;
    let command = queue_rpc_fixture_case(project.path(), "missing-locator")?;
    let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
    assert!(rpc.session_locator.is_none());
    prompt(
        &mut rpc,
        crate::extensions::PromptMode::Normal,
        "hold without locator",
    )?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentStarted)?;
    rpc.send_request(SessionCommand::Abort)?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentSettled)?;
    prompt(
        &mut rpc,
        crate::extensions::PromptMode::Normal,
        "fresh after stop",
    )?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentSettled)?;
    assert_eq!(
        fs::read_to_string(project.path().join("fixture-pids"))?
            .lines()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        2
    );
    rpc.terminate()?;
    Ok(())
}

#[test]
fn abort_reports_resume_readiness_failure_after_cleaning_up_both_processes() -> TestResult {
    let project = tempdir()?;
    let command = queue_rpc_fixture_case(project.path(), "second-readiness-fails")?;
    let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
    let error = rpc
        .send_request(SessionCommand::Abort)
        .expect_err("resume must fail");
    assert!(error.contains("Pi stopped; could not resume"), "{error}");
    assert!(error.contains("second readiness failed"), "{error}");
    assert!(
        rpc.child
            .lock()
            .map_err(|_| "child lock")?
            .try_wait()?
            .is_some()
    );
    assert_eq!(
        fs::read_to_string(project.path().join("fixture-pids"))?
            .lines()
            .count(),
        2
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn abort_reports_second_launch_failure_after_confirming_the_old_exit() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;

    let project = tempdir()?;
    let script = project.path().join("direct-queue-rpc.sh");
    fs::write(&script, include_str!("test_queue_rpc.sh"))?;
    let mut permissions = fs::metadata(&script)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&script, permissions)?;
    let command = AgentLaunchConfig {
        program: script.clone(),
        prefix_args: vec!["normal".into()],
        access_mode: HarnessAccessMode::Full,
        ..AgentLaunchConfig::default()
    };
    let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
    fs::remove_file(&script)?;
    let error = rpc
        .send_request(SessionCommand::Abort)
        .expect_err("resume must fail");
    assert!(error.contains("Pi stopped; could not resume"), "{error}");
    assert!(
        rpc.child
            .lock()
            .map_err(|_| "child lock")?
            .try_wait()?
            .is_some()
    );
    assert_eq!(
        fs::read_to_string(project.path().join("fixture-pids"))?
            .lines()
            .count(),
        1
    );
    Ok(())
}

#[test]
fn abort_discards_old_peer_reports_but_accepts_new_ones() -> TestResult {
    let project = tempdir()?;
    let command = queue_rpc_fixture(project.path())?;
    let registry = crate::core::CallerRegistry::shared();
    let sender = registry.issue(
        project.path(),
        crate::core::CallerProfile {
            backend: Backend::Pi,
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    sender.bind("peer-sender-session");
    let parent_id = registry.resolve(sender.token())?.worker_id;
    let mut rpc = PiRpcProcess::spawn_worker(
        &command,
        project.path(),
        SessionLaunch::New,
        "abort-peer-recipient".into(),
        "recipient".into(),
        Some((parent_id, "peer-sender-session".into())),
    )?;
    let concurrency = crate::core::WorkerConcurrency::new(1);
    let recipient_slot = concurrency.reserve()?;
    rpc.set_worker_slot(Some(recipient_slot.clone()));
    recipient_slot.release();
    let occupying_slot = concurrency.reserve()?;
    assert_eq!(
        registry.send(sender.token(), "recipient", "discard old report".into())?,
        Some("recipient".into())
    );
    rpc.send_request(SessionCommand::Abort)?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentSettled)?;
    drop(occupying_slot);
    assert_eq!(
        registry.send(sender.token(), "recipient", "deliver fresh report".into())?,
        Some("recipient".into())
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        let _ = rpc.try_next();
        if fs::read_to_string(project.path().join("fixture-requests"))
            .is_ok_and(|requests| requests.contains("deliver fresh report"))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let requests = fs::read_to_string(project.path().join("fixture-requests"))?;
    assert!(!requests.contains("discard old report"), "{requests}");
    assert!(requests.contains("deliver fresh report"), "{requests}");
    rpc.terminate()?;
    Ok(())
}

#[test]
#[ignore = "requires installed Pi; isolated local provider and tool, no network"]
fn installed_pi_apply_steering_resumes_after_tool_and_stream_abort() -> TestResult {
    let _mcp = DisabledMcp::new();
    for initial in ["hold tool", "hold stream"] {
        let project = tempdir()?;
        let command = installed_pi_fixture(project.path())?;
        let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
        rpc.request_and_wait(SessionCommand::ConfigureSteering)?;
        prompt(&mut rpc, crate::extensions::PromptMode::Normal, initial)?;
        wait_for_activity(
            &mut rpc,
            if initial == "hold tool" {
                crate::SessionActivityKind::ToolStarted
            } else {
                crate::SessionActivityKind::AgentStarted
            },
        )?;
        if initial == "hold tool" {
            let started = project.path().join("fixture-tool-started");
            let deadline = Instant::now() + Duration::from_secs(5);
            while !started.exists() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            assert!(started.exists(), "shell command did not start");
        }
        for _ in 0..2 {
            let id = prompt(&mut rpc, crate::extensions::PromptMode::Steer, "next")?;
            wait_for_response(&mut rpc, &id)?;
        }
        let apply = rpc.send_request(SessionCommand::ApplySteering)?;
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut acknowledged = false;
        let mut answered = false;
        let mut settled = false;
        while Instant::now() < deadline && !(acknowledged && settled) {
            match rpc.try_next() {
                Some(SessionEvent::Response(response))
                    if response.id.as_deref() == Some(&apply) =>
                {
                    response.result?;
                    acknowledged = true;
                }
                Some(SessionEvent::Activity(event)) => {
                    let value = event.value();
                    if value["type"] == "message_end" && value["message"]["role"] == "assistant" {
                        assert_ne!(value["message"]["stopReason"], "error", "{value}");
                        answered |= value["message"]["content"][0]["text"] == "done: next";
                    }
                    if value["type"] == "agent_settled" {
                        assert!(
                            answered,
                            "{initial}: exposed settlement before resumed answer"
                        );
                        settled = true;
                    }
                }
                Some(SessionEvent::Failure(error)) => return Err(error.into()),
                _ => thread::sleep(Duration::from_millis(5)),
            }
        }
        assert!(acknowledged && settled, "{initial}: handoff did not finish");
        let requests = fs::read_to_string(project.path().join("fixture-requests"))?;
        let last: Vec<String> =
            serde_json::from_str(requests.lines().last().ok_or("no requests")?)?;
        assert_eq!(last, [initial, "next", "next"]);
        rpc.terminate()?;
    }
    Ok(())
}

#[test]
#[ignore = "requires Pi 0.84.2 or newer installed on PATH"]
fn installed_pi_abort_and_apply_steering_control_real_stream_requests() -> TestResult {
    let _mcp = DisabledMcp::new();
    let project = tempdir()?;
    let command = installed_pi_fixture(project.path())?;
    let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
    rpc.request_and_wait(SessionCommand::ConfigureSteering)?;

    prompt(
        &mut rpc,
        crate::extensions::PromptMode::Normal,
        "installed persistence seed",
    )?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentSettled)?;

    prompt(
        &mut rpc,
        crate::extensions::PromptMode::Normal,
        "hold installed stop",
    )?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentStarted)?;
    wait_for_established_installed_request(&mut rpc, project.path(), "hold installed stop")?;
    let steering = prompt(
        &mut rpc,
        crate::extensions::PromptMode::Steer,
        "discard installed steering",
    )?;
    let follow_up = prompt(
        &mut rpc,
        crate::extensions::PromptMode::FollowUp,
        "discard installed follow-up",
    )?;
    wait_for_response(&mut rpc, &steering)?;
    wait_for_response(&mut rpc, &follow_up)?;
    rpc.send_request(SessionCommand::Abort)?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentSettled)?;
    let stopped_session = rpc.session_locator.clone();

    prompt(
        &mut rpc,
        crate::extensions::PromptMode::Normal,
        "hold installed apply",
    )?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentStarted)?;
    wait_for_established_installed_request(&mut rpc, project.path(), "hold installed apply")?;
    let queued = [
        prompt(
            &mut rpc,
            crate::extensions::PromptMode::FollowUp,
            "run installed queued",
        )?,
        prompt(
            &mut rpc,
            crate::extensions::PromptMode::FollowUp,
            "run installed second follow-up",
        )?,
        prompt(
            &mut rpc,
            crate::extensions::PromptMode::Steer,
            "run installed queued",
        )?,
        prompt(
            &mut rpc,
            crate::extensions::PromptMode::Steer,
            "run installed second steer",
        )?,
    ];
    for id in queued {
        wait_for_response(&mut rpc, &id)?;
    }
    rpc.send_request(SessionCommand::ApplySteering)?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentSettled)?;

    prompt(
        &mut rpc,
        crate::extensions::PromptMode::Normal,
        "installed submit again",
    )?;
    wait_for_activity(&mut rpc, crate::SessionActivityKind::AgentSettled)?;
    rpc.request_and_wait(SessionCommand::LoadState)?;
    assert_eq!(rpc.session_locator, stopped_session);
    let request_log = fs::read_to_string(project.path().join("fixture-requests"))?;
    let requests = request_log
        .lines()
        .map(serde_json::from_str::<Vec<String>>)
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        requests
            .last()
            .ok_or("installed Pi fixture made no requests")?
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "installed persistence seed",
            "hold installed stop",
            "hold installed apply",
            "run installed queued",
            "run installed second steer",
            "run installed queued",
            "run installed second follow-up",
            "installed submit again",
        ]
    );
    rpc.terminate()?;
    Ok(())
}

#[test]
fn process_starts_directly_in_the_project_directory() -> TestResult {
    crate::builtin_mcp::with_url_for_test("http://127.0.0.1:32123/mcp", || {
        let _mcp = crate::builtin_mcp::exclusive_for_test();
        let (temp, command) = fake("project-directory")?;
        let mut rpc = PiRpcProcess::spawn(&command, temp.path(), None)?;
        let process_project = fs::read_to_string(temp.path().join("process-project"))?;
        assert_eq!(
            fs::canonicalize(process_project)?,
            fs::canonicalize(temp.path())?,
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("process-mcp-url"))?,
            "http://127.0.0.1:32123/mcp"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("process-mcp-header"))?,
            "farcaster-caller"
        );
        assert!(!fs::read_to_string(temp.path().join("process-mcp-caller"))?.is_empty());
        assert!(!temp.path().join(".mcp.json").exists());
        rpc.terminate()?;
        Ok(())
    })
}

#[test]
fn child_process_omits_farcaster_mcp() -> TestResult {
    for launch in [
        SessionLaunch::New,
        SessionLaunch::Resume(Path::new("/sessions/parent.jsonl")),
        SessionLaunch::Fork(Path::new("/sessions/parent.jsonl")),
    ] {
        let (temp, command) = fake("project-directory")?;
        let resumed = temp.path().join("fake-session.jsonl");
        let launch = match launch {
            SessionLaunch::Resume(_) => SessionLaunch::Resume(&resumed),
            other => other,
        };
        let mut rpc = PiRpcProcess::spawn_worker(
            &command,
            temp.path(),
            launch,
            "child-worker".into(),
            "child".into(),
            None,
        )?;
        assert!(fs::read_to_string(temp.path().join("process-mcp-url"))?.is_empty());
        assert!(fs::read_to_string(temp.path().join("process-mcp-caller"))?.is_empty());
        rpc.terminate()?;
    }
    Ok(())
}

#[test]
fn catalog_process_disables_session_persistence() -> TestResult {
    let project = tempdir()?;
    let process = rpc_command(
        &AgentLaunchConfig {
            ..AgentLaunchConfig::default()
        },
        project.path(),
        SessionLaunch::Catalog,
    )?;
    assert!(
        process
            .get_args()
            .any(|argument| argument == "--no-session")
    );
    Ok(())
}

#[test]
fn fork_process_passes_the_source_session_to_pi() -> TestResult {
    let project = tempdir()?;
    let source = Path::new("/sessions/source session.jsonl");
    let process = rpc_command(
        &AgentLaunchConfig {
            ..AgentLaunchConfig::default()
        },
        project.path(),
        SessionLaunch::Fork(source),
    )?;
    let arguments = process.get_args().collect::<Vec<_>>();
    assert!(arguments.windows(2).any(|pair| pair == ["--mode", "rpc"]));
    assert!(
        !arguments
            .iter()
            .any(|argument| *argument == "--append-system-prompt")
    );
    assert_eq!(
        arguments.get(arguments.len().saturating_sub(2)..),
        Some([std::ffi::OsStr::new("--fork"), source.as_os_str()].as_slice())
    );
    Ok(())
}

#[test]
fn process_omits_builtin_mcp_when_disabled() -> TestResult {
    let _mcp = DisabledMcp::new();
    let project = tempdir()?;
    let extension = project.path().join("extension.mjs");
    let process = prepare_rpc(
        &AgentLaunchConfig::default(),
        project.path(),
        SessionLaunch::New,
        RpcMetadata {
            extension: &extension,
            is_worker: false,
            identity: None,
            parent_worker: None,
            parent_session: None,
            caller_token: "caller-1",
        },
    )?;
    assert!(
        !process
            .get_envs()
            .any(|(name, value)| name == "FARCASTER_MCP_URL" && value.is_some())
    );
    assert!(
        !process
            .get_envs()
            .any(|(name, value)| name == "FARCASTER_MCP_CALLER" && value.is_some())
    );
    Ok(())
}

#[test]
fn packaged_pi_path_wins_over_the_project_environment() {
    assert_eq!(
        pi_program(Some("/nix/store/pi/bin/pi".into())),
        PathBuf::from("/nix/store/pi/bin/pi")
    );
    assert_eq!(pi_program(None), PathBuf::from("pi"));
}

#[cfg(unix)]
#[test]
fn resolves_agent_symlink_to_a_fixed_executable() -> TestResult {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let root = tempdir()?;
    let executable = root.path().join("agent");
    let bin = root.path().join("bin");
    fs::create_dir(&bin)?;
    fs::write(&executable, b"#!/usr/bin/env node\n")?;
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))?;
    symlink(&executable, bin.join("agent"))?;

    let search_path = std::env::join_paths([&bin])?;
    let resolved = resolve_agent_program(Path::new("agent"), root.path(), Some(&search_path))?;
    assert_eq!(resolved, executable.canonicalize()?);
    Ok(())
}

#[test]
fn pi_without_a_sandbox_adapter_leaves_extension_settings_alone() -> TestResult {
    let project = tempdir()?;
    let pi = project.path().join("pi");
    fs::write(&pi, b"#!/bin/sh\nexit 0\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&pi, fs::Permissions::from_mode(0o700))?;
    }
    let prepare = |access_mode| {
        rpc_command(
            &AgentLaunchConfig {
                program: pi.clone(),
                access_mode,
                ..AgentLaunchConfig::default()
            },
            project.path(),
            SessionLaunch::New,
        )
    };

    let sandboxed = prepare(HarnessAccessMode::Sandboxed)?;
    assert_eq!(sandboxed.get_program(), pi.canonicalize()?);
    assert!(
        !sandboxed
            .get_envs()
            .any(|(name, _)| name == "PI_NONO_DISABLED")
    );

    let full = prepare(HarnessAccessMode::Full)?;
    assert!(!full.get_envs().any(|(name, _)| name == "PI_NONO_DISABLED"));
    assert!(
        !full
            .get_args()
            .any(|arg| arg.to_string_lossy().starts_with("--sandbox"))
    );
    Ok(())
}

#[test]
fn request_and_wait_confirms_configuration_before_returning() -> TestResult {
    let (temp, command) = fake("normal")?;
    let mut rpc = PiRpcProcess::spawn(&command, temp.path(), None)?;
    let response = rpc.request_and_wait(SessionCommand::SelectReasoning {
        level: "medium".into(),
    })?;
    assert_eq!(
        response.operation(),
        crate::SessionOperation::SelectReasoning
    );
    rpc.terminate()?;
    Ok(())
}

#[test]
fn sandbox_adapter_detects_each_launch_and_blocks_failed_control() -> TestResult {
    use HarnessAccessMode::{Auto, Full, Sandboxed};
    for (requested, expected) in [(Auto, Sandboxed), (Sandboxed, Sandboxed), (Full, Full)] {
        let (temp, mut command) = fake("sandbox-ready")?;
        command.access_mode = requested;
        let mut process = PiRpcProcess::spawn(&command, temp.path(), None)?;
        assert_eq!(process.confirmed_sandbox_mode(), Some(expected));
        process.caller_identity.bind("parent-session");
        assert_eq!(
            crate::CallerRegistry::shared()
                .resolve(process.caller_identity.token())?
                .access_mode,
            expected
        );
        assert!(!temp.path().join("agent-prompts").exists());
        process.request_and_wait(SessionCommand::Prompt {
            mode: crate::extensions::PromptMode::Normal,
            message: "user prompt".into(),
            images: vec![],
        })?;
        assert!(fs::read_to_string(temp.path().join("agent-prompts"))?.contains("user prompt"));
        process.terminate()?;
    }
    for (case, message) in [
        ("sandbox-failed", "unavailable"),
        ("sandbox-stale", "Stale"),
        ("sandbox-wrong-mode", "requested"),
        ("sandbox-rejected", "rejected"),
    ] {
        let (temp, mut command) = fake(case)?;
        command.access_mode = Sandboxed;
        let error = PiRpcProcess::spawn(&command, temp.path(), None)
            .err()
            .ok_or("sandbox startup unexpectedly succeeded")?;
        assert!(error.contains(message), "{case}: {error}");
        assert!(!temp.path().join("agent-prompts").exists());
    }
    Ok(())
}

#[test]
fn missing_sandbox_registers_the_caller_as_full_access() -> TestResult {
    let (temp, mut command) = fake("normal")?;
    command.access_mode = HarnessAccessMode::Auto;
    let mut process = PiRpcProcess::spawn(&command, temp.path(), None)?;
    assert_eq!(process.confirmed_sandbox_mode(), None);
    assert_eq!(
        crate::SessionTransport::sandbox_mode(&process),
        Some(HarnessAccessMode::Full)
    );
    process.caller_identity.bind("parent-session");
    assert_eq!(
        crate::CallerRegistry::shared()
            .resolve(process.caller_identity.token())?
            .access_mode,
        HarnessAccessMode::Full
    );
    process.terminate()?;
    Ok(())
}

#[test]
fn sandbox_control_needs_more_than_a_successful_rpc_response() -> TestResult {
    let (temp, command) = fake("quiet")?;
    let mut process = PiRpcProcess::spawn(&command, temp.path(), None)?;
    let result = process.confirm_control(
        serde_json::json!({"type":"get_state"}),
        Duration::from_millis(50),
        |_| None,
    );
    assert!(
        result
            .expect_err("sandbox confirmation must fail")
            .contains("did not confirm")
    );
    Ok(())
}

#[test]
fn sandbox_mode_drift_revokes_confirmation_and_blocks_further_prompts() -> TestResult {
    let (temp, mut command) = fake("sandbox-ready")?;
    command.access_mode = HarnessAccessMode::Sandboxed;
    let mut process = PiRpcProcess::spawn(&command, temp.path(), None)?;
    let report = serde_json::json!({"version":1,"requestId":"external","files":"full","network":"full","success":true});
    let event = process.route(ReaderItem::Wire(Box::new(Ok(PiWireMessage::ExtensionUi(
        crate::extensions::ExtensionUiRequest::SetStatus {
            id: "mode-change".into(),
            key: "\u{1f}pi-gpui-sandbox-mode\u{1f}".into(),
            text: Some(report.to_string()),
        },
    )))));
    assert!(matches!(event, SessionEvent::Failure(_)));
    assert_eq!(process.confirmed_sandbox_mode(), None);
    assert_eq!(crate::SessionTransport::sandbox_mode(&process), None);
    assert!(
        process
            .send_request(SessionCommand::Prompt {
                mode: crate::extensions::PromptMode::Normal,
                message: "must not execute".into(),
                images: vec![],
            })
            .is_err()
    );
    assert!(!temp.path().join("agent-prompts").exists());
    Ok(())
}

#[test]
fn sandbox_worker_discovers_control_and_rechecks_after_fork() -> TestResult {
    let (temp, mut command) = fake("sandbox-ready")?;
    command.access_mode = HarnessAccessMode::Sandboxed;
    let mut worker = PiRpcProcess::spawn_worker(
        &command,
        temp.path(),
        SessionLaunch::New,
        "sandbox-child".into(),
        "child".into(),
        None,
    )?;
    assert_eq!(
        worker.confirmed_sandbox_mode(),
        Some(HarnessAccessMode::Sandboxed)
    );
    worker.request_and_wait(SessionCommand::ForkAt {
        entry_id: "branch".into(),
    })?;
    assert_eq!(
        worker.confirmed_sandbox_mode(),
        Some(HarnessAccessMode::Sandboxed)
    );
    assert!(!temp.path().join("agent-prompts").exists());
    assert_eq!(
        fs::read_to_string(temp.path().join("sandbox-controls"))?
            .lines()
            .count(),
        2
    );
    Ok(())
}

#[test]
fn sandbox_discovery_ignores_unrelated_commands_without_sending_control() -> TestResult {
    for case in [
        "quiet",
        "sandbox-missing",
        "sandbox-template",
        "sandbox-no-source",
        "sandbox-unrelated",
    ] {
        let (temp, mut command) = fake(case)?;
        let mut process = PiRpcProcess::spawn(&command, temp.path(), None)?;
        assert_eq!(process.sandbox_adapter_id(), None, "{case}");
        assert_eq!(process.confirmed_sandbox_mode(), None, "{case}");
        assert!(!temp.path().join("sandbox-controls").exists(), "{case}");
        assert!(!temp.path().join("agent-prompts").exists(), "{case}");
        process.terminate()?;
        command.access_mode = HarnessAccessMode::Sandboxed;
        assert!(
            PiRpcProcess::spawn(&command, temp.path(), None).is_err(),
            "{case}"
        );
    }
    Ok(())
}

#[test]
fn sandbox_discovery_uses_the_qualified_command_name() -> TestResult {
    let (temp, mut command) = fake("sandbox-collision")?;
    command.access_mode = HarnessAccessMode::Sandboxed;
    let process = PiRpcProcess::spawn(&command, temp.path(), None)?;
    assert_eq!(
        process.confirmed_sandbox_mode(),
        Some(HarnessAccessMode::Sandboxed)
    );
    assert!(fs::read_to_string(temp.path().join("sandbox-controls"))?.contains("/sandbox-mode:2 "));
    assert!(!temp.path().join("agent-prompts").exists());
    Ok(())
}

#[test]
#[ignore = "requires FARCASTER_TEST_PI and FARCASTER_TEST_PI_NONO plus native sandbox support"]
fn live_pi_nono_sandbox_discovery_without_inference() -> TestResult {
    let program = std::env::var("FARCASTER_TEST_PI")?;
    let extension = std::env::var("FARCASTER_TEST_PI_NONO")?;
    for mode in [
        None,
        Some(HarnessAccessMode::Sandboxed),
        Some(HarnessAccessMode::Full),
    ] {
        let temp = tempdir()?;
        let mut args = vec![
            format!(
                "PI_CODING_AGENT_DIR={}",
                temp.path().join("agent").display()
            ),
            "PI_OFFLINE=1".into(),
            "PI_NONO_DISABLED=0".into(),
            program.clone(),
            "--no-session".into(),
            "--no-extensions".into(),
        ];
        if mode.is_some() {
            args.extend(["--extension".into(), extension.clone()]);
        }
        let command = AgentLaunchConfig {
            program: "/usr/bin/env".into(),
            prefix_args: args,
            access_mode: mode.unwrap_or(HarnessAccessMode::Full),
            ..Default::default()
        };
        // Use the worker launch path to omit the unrelated MCP extension flag.
        let mut process = PiRpcProcess::spawn_worker(
            &command,
            temp.path(),
            SessionLaunch::Catalog,
            "sandbox-probe".into(),
            "sandbox-probe".into(),
            None,
        )?;
        assert_eq!(process.sandbox_adapter_id(), mode.map(|_| "pi-nono"));
        assert_eq!(process.confirmed_sandbox_mode(), mode);
        let response = process.request_and_wait(SessionCommand::LoadState)?;
        let Ok(crate::SessionResponsePayload::LoadState(state)) = response.result else {
            panic!("state");
        };
        assert_eq!(state.message_count, 0);
        while let Some(event) = process.try_next() {
            assert!(!matches!(event, SessionEvent::Activity(ref activity)
                if matches!(activity.kind(), crate::SessionActivityKind::AgentStarted)));
        }
        process.terminate()?;
    }
    Ok(())
}

#[test]
fn handshake_routes_async_event_and_correlates_unique_ids() -> TestResult {
    let (temp, command) = fake("normal")?;
    let mut rpc = PiRpcProcess::spawn(&command, temp.path(), None)?;
    assert!(
        matches!(rpc.try_next(), Some(SessionEvent::Activity(value)) if value.kind() == &crate::SessionActivityKind::AgentStarted)
    );
    let first = rpc.send_command(serde_json::json!({"type":"get_messages"}))?;
    let second = rpc.send_command(serde_json::json!({"type":"get_state"}))?;
    let stats = rpc.send_command(serde_json::json!({"type":"get_session_stats"}))?;
    assert_ne!(first, second);
    assert_ne!(second, stats);
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut responses = 0;
    let mut context_shape = false;
    while Instant::now() < deadline && responses < 3 {
        if let Some(SessionEvent::Response(response)) = rpc.try_next() {
            responses += 1;
            context_shape |= matches!(response.result,
                Ok(crate::SessionResponsePayload::LoadUsage(usage))
                if usage.context_usage.is_some_and(|context|
                    context.tokens == Some(4096) && context.context_window == 8192 && context.percent == Some(50.0))
            );
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(responses, 3);
    assert!(context_shape);
    Ok(())
}

#[test]
fn peer_message_steers_a_busy_session_without_waiting_for_settlement() -> TestResult {
    let (temp, command) = fake("peer-delivery")?;
    let registry = crate::core::CallerRegistry::shared();
    let sender = registry.issue(
        temp.path(),
        crate::core::CallerProfile {
            backend: Backend::Pi,
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    sender.bind("sender-session");
    let parent_id = registry.resolve(sender.token())?.worker_id;
    let mut rpc = PiRpcProcess::spawn_worker(
        &command,
        temp.path(),
        SessionLaunch::New,
        "recipient-worker".into(),
        "recipient".into(),
        Some((parent_id, "sender-session".into())),
    )?;
    rpc.send_request(SessionCommand::Prompt {
        mode: crate::extensions::PromptMode::Normal,
        message: "keep working".into(),
        images: Vec::new(),
    })?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut started = false;
    while Instant::now() < deadline {
        if matches!(
            rpc.try_next(),
            Some(SessionEvent::Activity(activity))
                if activity.kind() == &crate::SessionActivityKind::AgentStarted
        ) {
            started = true;
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(started, "fake Pi did not start its turn");

    assert_eq!(
        registry.send(sender.token(), "recipient", "peer update".into())?,
        Some("recipient".into())
    );

    let log_path = temp.path().join("peer-delivery.log");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut log = String::new();
    while Instant::now() < deadline {
        let _ = rpc.try_next();
        log = fs::read_to_string(&log_path)?;
        if log.contains("\"type\":\"steer\"") {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(log.contains("\"type\":\"steer\""), "{log}");
    assert!(log.contains("peer update"), "{log}");
    rpc.terminate()?;
    Ok(())
}

#[test]
fn eof_with_pending_request_is_failure_and_stderr_is_visible() -> TestResult {
    let (temp, command) = fake("eof")?;
    let mut rpc = PiRpcProcess::spawn(&command, temp.path(), None)?;
    rpc.send_command(serde_json::json!({"type":"get_messages"}))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut failure = String::new();
    while Instant::now() < deadline && failure.is_empty() {
        if let Some(SessionEvent::Failure(error)) = rpc.try_next() {
            failure = error;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(failure.contains("pending request"));
    assert!(failure.contains("exit code 7"));
    assert!(failure.contains("fake stderr before exit"));
    Ok(())
}

#[test]
fn failed_readiness_is_reported() -> TestResult {
    let (temp, command) = fake("bad-handshake")?;
    let error = PiRpcProcess::spawn(&command, temp.path(), None)
        .err()
        .unwrap_or_default();
    assert!(error.contains("readiness"), "{error}");
    Ok(())
}

#[test]
fn stdout_eof_waits_for_delayed_final_stderr() -> TestResult {
    let (temp, command) = fake("delayed-stderr")?;
    let mut rpc = PiRpcProcess::spawn(&command, temp.path(), None)?;
    rpc.send_command(serde_json::json!({"type":"get_messages"}))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut failure = String::new();
    while Instant::now() < deadline && failure.is_empty() {
        if let Some(SessionEvent::Failure(error)) = rpc.try_next() {
            failure = error;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(failure.contains("delayed final stderr"), "{failure}");
    assert!(failure.contains("exit code 8"), "{failure}");
    Ok(())
}

#[test]
fn readiness_rejects_a_command_mismatch_for_the_right_id() -> TestResult {
    let (temp, command) = fake("mismatch-handshake")?;
    let error = PiRpcProcess::spawn(&command, temp.path(), None)
        .err()
        .unwrap_or_default();
    assert!(error.contains("expected get_state"));
    Ok(())
}

#[test]
fn ordinary_response_rejects_a_command_mismatch_for_the_right_id() -> TestResult {
    let (temp, command) = fake("mismatch-response")?;
    let mut rpc = PiRpcProcess::spawn(&command, temp.path(), None)?;
    rpc.send_command(serde_json::json!({"type":"get_messages"}))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut failure = String::new();
    while Instant::now() < deadline && failure.is_empty() {
        if let Some(SessionEvent::Failure(error)) = rpc.try_next() {
            failure = error;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(failure.contains("expected get_messages"));
    Ok(())
}

#[test]
fn terminate_reaps_graceful_and_term_ignoring_children() -> TestResult {
    for case_name in ["normal", "ignore-term"] {
        let (temp, command) = fake(case_name)?;
        let mut rpc = PiRpcProcess::spawn(&command, temp.path(), None)?;
        let started = Instant::now();
        rpc.terminate()?;
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(
            rpc.child
                .lock()
                .map_err(|_| "poisoned")?
                .try_wait()?
                .is_some()
        );
    }
    Ok(())
}

#[test]
fn stamp_parent_session_rewrites_the_header_in_place() -> TestResult {
    let temp = tempdir()?;
    let path = temp.path().join("child.jsonl");
    fs::write(
        &path,
        concat!(
            r#"{"type":"session","version":3,"id":"child-1","cwd":"/project"}"#,
            "\n",
            r#"{"type":"message","id":"m1"}"#,
            "\n",
        ),
    )?;
    stamp_parent_session(&path, "/sessions/parent.jsonl")?;
    let contents = fs::read_to_string(&path)?;
    let header_line = contents.lines().next().ok_or("missing session header")?;
    let header: serde_json::Value = serde_json::from_str(header_line)?;
    assert_eq!(
        header["parentSession"].as_str(),
        Some("/sessions/parent.jsonl")
    );
    assert!(contents.contains(r#""id":"m1""#));
    Ok(())
}

#[test]
fn inherited_child_does_not_stamp_the_parent_before_forking() -> TestResult {
    let (temp, command) = fake("deferred-session")?;
    let path = temp.path().canonicalize()?.join("fake-session.jsonl");
    let contents = "{\"type\":\"session\",\"version\":3,\"id\":\"parent\"}\n";
    fs::write(&path, contents)?;
    let registry = crate::CallerRegistry::shared();
    let parent = registry.issue(
        temp.path(),
        crate::core::CallerProfile {
            backend: Backend::Pi,
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    let locator = path.to_string_lossy().into_owned();
    parent.bind(locator.clone());
    let parent_id = registry.resolve(parent.token())?.worker_id;
    let mut rpc = PiRpcProcess::spawn_worker(
        &command,
        temp.path(),
        SessionLaunch::Resume(&path),
        "inherited-child".into(),
        "review".into(),
        Some((parent_id, locator.clone())),
    )?;
    assert_eq!(fs::read_to_string(&path)?, contents);
    assert_eq!(rpc.parent_session.as_deref(), Some(locator.as_str()));
    assert!(rpc.pending_parent_stamp.is_none());
    rpc.terminate()?;
    Ok(())
}

#[test]
fn child_parent_stamp_retries_after_pi_reports_an_uncreated_session_file() -> TestResult {
    let (temp, command) = fake("deferred-session")?;
    let path = temp.path().canonicalize()?.join("fake-session.jsonl");
    let registry = crate::CallerRegistry::shared();
    let parent = registry.issue(
        temp.path(),
        crate::core::CallerProfile {
            backend: Backend::Pi,
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    parent.bind("/sessions/parent.jsonl");
    let parent_id = registry.resolve(parent.token())?.worker_id;
    let mut rpc = PiRpcProcess::spawn_worker(
        &command,
        temp.path(),
        SessionLaunch::New,
        "child-worker".into(),
        "review".into(),
        Some((parent_id, "/sessions/parent.jsonl".into())),
    )?;
    assert!(!path.exists());
    assert_eq!(rpc.pending_parent_stamp.as_deref(), Some(path.as_path()));

    fs::write(
        &path,
        r#"{"type":"session","version":3,"id":"child-1","cwd":"/project"}
"#,
    )?;
    let _ = rpc.route(ReaderItem::Stderr(String::new()));

    let header: serde_json::Value = serde_json::from_str(
        fs::read_to_string(&path)?
            .lines()
            .next()
            .ok_or("missing session header")?,
    )?;
    assert_eq!(
        header["parentSession"].as_str(),
        Some("/sessions/parent.jsonl")
    );
    assert!(rpc.pending_parent_stamp.is_none());
    rpc.terminate()?;
    Ok(())
}

#[test]
fn resume_readiness_requires_the_requested_session_file() -> TestResult {
    let (temp, command) = fake("fixed-session")?;
    let expected = temp.path().join("fake-session.jsonl");
    let mut resumed = PiRpcProcess::spawn(&command, temp.path(), Some(&expected))?;
    resumed.terminate()?;
    let wrong = temp.path().join("different-session.jsonl");
    let result = PiRpcProcess::spawn(&command, temp.path(), Some(&wrong));
    match result {
        Err(error) => assert!(
            error.contains("did not resume the requested session"),
            "{error}"
        ),
        Ok(mut process) => {
            process.terminate()?;
            panic!("readiness accepted a different session file");
        }
    }
    Ok(())
}

#[test]
fn only_the_pi_adapter_selects_the_pi_executable() {
    let neutral = AgentLaunchConfig::default();
    assert!(neutral.program.as_os_str().is_empty());
    assert!(resolve_agent_program(&neutral.program, Path::new("/project"), None).is_err());
    assert!(
        !launch_configuration(&neutral)
            .program
            .as_os_str()
            .is_empty()
    );
    let explicit = AgentLaunchConfig {
        program: PathBuf::from("/custom/pi"),
        ..neutral
    };
    assert_eq!(launch_configuration(&explicit).program, explicit.program);
}

#[test]
fn malformed_catalog_fails_its_request_without_poisoning_the_transport() -> TestResult {
    let (temp, command) = fake("malformed-catalog")?;
    let mut rpc = PiRpcProcess::spawn(&command, temp.path(), None)?;
    let error = rpc
        .request_and_wait(SessionCommand::ListModels)
        .expect_err("invalid model catalog");
    assert!(error.contains("ListModels"), "{error}");
    let response = rpc.request_and_wait(SessionCommand::LoadState)?;
    assert!(matches!(
        response.result,
        Ok(crate::SessionResponsePayload::LoadState(_))
    ));
    rpc.terminate()?;
    Ok(())
}

#[path = "process_receipt_tests.rs"]
mod receipts;

#[path = "process_cancellation_tests.rs"]
mod cancellations;
