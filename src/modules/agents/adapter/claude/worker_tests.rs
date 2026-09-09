use super::*;
use std::thread;
use std::time::{Duration, Instant};

const FIXTURES: &str =
    include_str!("../../../../../crates/claude-sdk-types/fixtures/protocol.json");

#[test]
fn cancellation_receipt_settles_only_the_named_active_prompt() {
    let (directory, command) = setup();
    let mut session = session(&command, directory.path());
    session
        .send("hold".into(), WorkerSendMode::Prompt)
        .expect("test operation should succeed");
    let active = session
        .active_uuid
        .clone()
        .expect("test operation should succeed");
    for (request, cancelled, settled) in [
        ("wrong", "another-prompt", false),
        ("right", active.as_str(), true),
    ] {
        session.interrupts.insert(request.into());
        session
            .receive(
                decode(json!({"type":"control_response","response":{
                    "subtype":"success","request_id":request,
                    "response":{"still_queued":[],"cancelled":[cancelled]}
                }}))
                .expect("test operation should succeed"),
            )
            .expect("test operation should succeed");
        assert_eq!(!session.active, settled);
    }
    assert_eq!(
        session
            .events
            .pending
            .iter()
            .filter(|event| matches!(event, WorkerEvent::Settled { .. }))
            .count(),
        1
    );
    session.close().expect("test operation should succeed");
}

#[test]
fn interrupted_result_settles_but_real_execution_errors_fail() {
    let (directory, command) = setup();
    let mut session = session(&command, directory.path());
    for (reason, stopped) in [
        ("aborted_streaming", true),
        ("aborted_tools", true),
        ("api_error", false),
    ] {
        session
            .send("hold".into(), WorkerSendMode::Prompt)
            .expect("test operation should succeed");
        session.events.pending.clear();
        let mut result = fixture("SDKResultSuccess");
        result["subtype"] = json!("error_during_execution");
        result["is_error"] = json!(true);
        result["errors"] = json!(["execution stopped"]);
        result["terminal_reason"] = json!(reason);
        session
            .receive(decode(result).expect("test operation should succeed"))
            .expect("test operation should succeed");
        assert_eq!(
            session
                .events
                .pending
                .iter()
                .any(|event| matches!(event, WorkerEvent::Settled { .. })),
            stopped
        );
        assert_eq!(
            session
                .events
                .pending
                .iter()
                .any(|event| matches!(event, WorkerEvent::Failed(_))),
            !stopped
        );
    }
    session.close().expect("test operation should succeed");
}

fn fixture(name: &str) -> Value {
    serde_json::from_str::<Vec<Value>>(FIXTURES)
        .expect("test operation should succeed")
        .into_iter()
        .find(|fixture| fixture["rust_type"] == name)
        .expect("test operation should succeed")["value"]
        .clone()
}

const SCRIPT: &str = r#"#!/bin/sh
printf '%s\n' "$@" > "$0.args"
reply() { printf '{"type":"control_response","response":{"subtype":"success","request_id":"%s","response":%s}}\n' "$id" "$1"; }
printf '%s\n' '{"type":"command_lifecycle"}'
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$0.requests"
  id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"subtype":"initialize"'*) reply '{"commands":[],"agents":[],"output_style":"default","available_output_styles":["default"],"models":[{"value":"fixture","displayName":"Fixture","description":"test","supportsEffort":true,"supportedEffortLevels":["low","high"]}],"account":{}}' ;;
    *'"subtype":"interrupt"'*) reply '{}'; cat "$0.result" ;;
    *'"type":"control_request"'*) reply '{}' ;;
    *'"type":"control_response"'*) printf '%s\n' '{"type":"command_lifecycle"}'; cat "$0.turn"; cat "$0.result" ;;
    *'"type":"user"'*)
      case "$line" in
        *'hold'*) ;;
        *'crash'*) exit 7 ;;
        *) printf '%s\n' '{"type":"control_request","request_id":"permission","request":{"subtype":"can_use_tool","tool_name":"Read","input":{"file_path":905},"tool_use_id":"tool-1"}}' ;;
      esac ;;
    *) exit 2 ;;
  esac
done
"#;

fn setup() -> (tempfile::TempDir, AgentLaunchConfig) {
    let directory = tempfile::tempdir().expect("test operation should succeed");
    let script = directory.path().join("claude-fixture");
    std::fs::write(&script, SCRIPT).expect("test operation should succeed");
    let mut result = fixture("SDKResultSuccess");
    result["result"] = json!("fixture ok");
    std::fs::write(script.with_extension("result"), format!("{result}\n"))
        .expect("test operation should succeed");
    let mut assistant = fixture("SDKAssistantMessage");
    let tool = assistant["message"]["content"][0].clone();
    assistant["message"]["content"] =
        json!([{"type":"text","text":"fixture ok","citations":null},tool]);
    let delta = json!({"type":"stream_event", "event":{"type":"content_block_delta","index":0,
        "delta":{"type":"text_delta","text":"fixture "}}, "uuid":"delta", "session_id":"one", "parent_tool_use_id":null});
    let replay = fixture("SDKUserMessageReplay");
    std::fs::write(
        script.with_extension("turn"),
        format!("{delta}\n{assistant}\n{replay}\n"),
    )
    .expect("test operation should succeed");
    let mut command = AgentLaunchConfig::test_script(&script, Vec::new());
    command.access_mode = HarnessAccessMode::Sandboxed;
    (directory, command)
}

fn session(command: &AgentLaunchConfig, project: &Path) -> ClaudeSession {
    let caller = CallerRegistry::shared().issue(
        project,
        CallerProfile {
            backend: BACKEND.into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    let id = "00000000-0000-4000-8000-000000000001";
    let process = Process::spawn(command, project, id, false, None, None, true)
        .expect("test operation should succeed");
    attach(process, caller, id, command.access_mode)
        .expect("test operation should succeed")
        .0
}

fn until(
    session: &mut ClaudeSession,
    mut done: impl FnMut(&WorkerEvent) -> bool,
) -> Vec<WorkerEvent> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut events = Vec::new();
    while Instant::now() < deadline {
        if let Some(event) = session.poll() {
            let stop = done(&event);
            events.push(event);
            if stop {
                return events;
            }
        } else {
            thread::sleep(Duration::from_millis(5));
        }
    }
    panic!("Claude fixture did not reach expected state: {events:?}");
}

#[test]
fn cli_round_trip_streams_once_preserves_arguments_and_queues_turns() {
    let (directory, command) = setup();
    let mut session = session(&command, directory.path());
    session
        .select_model(BACKEND, "fixture")
        .expect("test operation should succeed");
    session
        .select_effort("high")
        .expect("test operation should succeed");
    assert!(session.select_effort("max").is_err());
    assert!(session.select_mode("bypassPermissions").is_err());
    session
        .select_mode("acceptEdits")
        .expect("test operation should succeed");
    session
        .send("hello".into(), WorkerSendMode::Prompt)
        .expect("test operation should succeed");
    session
        .send("second".into(), WorkerSendMode::Queue)
        .expect("test operation should succeed");
    for _ in 0..2 {
        let events = until(&mut session, |event| {
            matches!(event, WorkerEvent::NeedsInput(_))
        });
        let WorkerEvent::NeedsInput(input) = events.last().expect("test operation should succeed")
        else {
            unreachable!()
        };
        assert!(input.prompt.contains("905"));
        assert_eq!(input.options[0], "Deny");
        session
            .respond(WorkerInputResponse {
                id: input.id.clone(),
                value: Some("Allow".into()),
                cancel: false,
            })
            .expect("test operation should succeed");
        let events = until(&mut session, |event| {
            matches!(event, WorkerEvent::Settled { .. } | WorkerEvent::Failed(_))
        });
        assert!(
            matches!(events.last(),Some(WorkerEvent::Settled {output}) if output=="fixture ok"),
            "{events:?}"
        );
        let text = events
            .iter()
            .filter_map(|event| match event {
                WorkerEvent::Activity(WorkerActivity::TextDelta { delta, .. }) => {
                    Some(delta.as_str())
                }
                _ => None,
            })
            .collect::<String>();
        assert_eq!(text, "fixture ok");
        assert!(events.iter().any(|event|matches!(event,WorkerEvent::Activity(WorkerActivity::ToolStarted {args,metadata,..}) if args["file_path"]==905 && metadata.targets.is_empty())));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkerEvent::Activity(WorkerActivity::ToolFinished { is_error: true, .. })
        )));
    }
    session.close().expect("test operation should succeed");
    let requests = std::fs::read_to_string(directory.path().join("claude-fixture.requests"))
        .expect("test operation should succeed");
    assert!(requests.contains("\"updatedInput\":{\"file_path\":905}"));
    assert!(requests.contains("\"effortLevel\":\"high\""));
}

#[test]
fn interrupt_drops_queue_and_process_exit_fails_once() {
    let (directory, command) = setup();
    let mut session = session(&command, directory.path());
    session
        .send("hold".into(), WorkerSendMode::Prompt)
        .expect("test operation should succeed");
    session
        .send("not delivered".into(), WorkerSendMode::Queue)
        .expect("test operation should succeed");
    session.abort().expect("test operation should succeed");
    until(&mut session, |event| {
        matches!(event, WorkerEvent::Settled { .. })
    });
    assert!(session.queued.is_empty());
    session
        .send("crash".into(), WorkerSendMode::Prompt)
        .expect("test operation should succeed");
    until(&mut session, |event| {
        matches!(event, WorkerEvent::Failed(_))
    });
    assert!(session.closed);
    assert!(session.poll().is_none());
    assert!(
        session
            .send("retry".into(), WorkerSendMode::Prompt)
            .is_err()
    );
}

#[test]
fn cli_launch_and_image_envelopes_are_source_typed() {
    for (access, permission_mode) in [
        (HarnessAccessMode::Sandboxed, "--permission-mode=default"),
        (HarnessAccessMode::Auto, "--permission-mode=auto"),
        (
            HarnessAccessMode::Full,
            "--permission-mode=bypassPermissions",
        ),
    ] {
        let mut command = std::process::Command::new("claude");
        super::super::process::configure(&mut command, access, "id", true, None, false);
        let args = command
            .get_args()
            .map(|arg| arg.to_str().expect("test operation should succeed"))
            .collect::<Vec<_>>();
        assert!(args.contains(&"--resume=id"));
        assert!(args.contains(&"--no-session-persistence"));
        assert!(args.contains(&"--permission-prompt-tool"));
        assert!(args.contains(&permission_mode));
        assert!(!args.contains(&"--mcp-config"));
        assert_eq!(
            args.contains(&"--allow-dangerously-skip-permissions"),
            access == HarnessAccessMode::Full
        );
    }
    let message = prompt(
        "id",
        "see image",
        vec![crate::protocol::PromptImage::new(
            "YWJj".into(),
            "image/png".into(),
        )],
    )
    .expect("test operation should succeed");
    let value = serde_json::to_value(message).expect("test operation should succeed");
    assert_eq!(
        value["message"]["content"][1]["source"]["media_type"],
        "image/png"
    );
    assert!(
        prompt(
            "id",
            "image",
            vec![crate::protocol::PromptImage::new(
                "YWJj".into(),
                "image/svg+xml".into()
            )]
        )
        .is_err()
    );
    let (factories, _) = super::super::super::worker_factories(AgentLaunchConfig::default());
    assert!(factories.contains_key("claude"));
    assert!(!factories.contains_key("claude-acp"));
}

#[test]
fn catalog_probe_and_main_resume_launch_without_sending_a_prompt() {
    let (directory, command) = setup();
    let metadata =
        load_configuration(&command, directory.path()).expect("test operation should succeed");
    assert_eq!(metadata.models[0]["id"], "fixture");
    let args = std::fs::read_to_string(directory.path().join("claude-fixture.args"))
        .expect("test operation should succeed");
    assert!(args.lines().any(|arg| arg == "--no-session-persistence"));
    assert!(!args.lines().any(|arg| arg == "--mcp-config"));
    let id = "00000000-0000-4000-8000-000000000001";
    let launch = SessionLaunch {
        harness: BACKEND.into(),
        session_id: None,
        project: directory.path().into(),
        start: SessionStart::Resume(main_session::external_session_path(
            directory.path(),
            BACKEND,
            id,
        )),
        wake: None,
    };
    let (mut worker, locator, _) =
        spawn_main(&command, &launch).expect("test operation should succeed");
    assert_eq!(locator, id);
    worker.close().expect("test operation should succeed");
    let args = std::fs::read_to_string(directory.path().join("claude-fixture.args"))
        .expect("test operation should succeed");
    assert!(args.lines().any(|arg| arg == format!("--resume={id}")));
    assert!(!args.lines().any(|arg| arg == "--no-session-persistence"));
    if super::super::super::farcaster_mcp::enabled() {
        assert!(args.lines().any(|arg| arg == "--mcp-config"));
        assert!(args.contains("farcaster-caller"));
    }
    let requests = std::fs::read_to_string(directory.path().join("claude-fixture.requests"))
        .expect("test operation should succeed");
    assert!(!requests.contains("\"type\":\"user\""));
}

#[test]
fn cancelling_an_approval_denies_without_changing_tool_input() {
    let (directory, command) = setup();
    let mut session = session(&command, directory.path());
    session
        .send("hello".into(), WorkerSendMode::Prompt)
        .expect("test operation should succeed");
    let events = until(&mut session, |event| {
        matches!(event, WorkerEvent::NeedsInput(_))
    });
    let WorkerEvent::NeedsInput(input) = events.last().expect("test operation should succeed")
    else {
        unreachable!()
    };
    session
        .respond(WorkerInputResponse {
            id: input.id.clone(),
            value: Some("Allow".into()),
            cancel: true,
        })
        .expect("test operation should succeed");
    until(&mut session, |event| {
        matches!(event, WorkerEvent::Settled { .. } | WorkerEvent::Failed(_))
    });
    session.close().expect("test operation should succeed");
    let requests = std::fs::read_to_string(directory.path().join("claude-fixture.requests"))
        .expect("test operation should succeed");
    assert!(requests.contains("\"behavior\":\"deny\""));
    assert!(!requests.contains("\"behavior\":\"allow\""));
}

#[test]
fn permission_modes_match_launch_access_and_exclude_plan() {
    for (access, initial) in [
        (HarnessAccessMode::Sandboxed, "default"),
        (HarnessAccessMode::Auto, "auto"),
        (HarnessAccessMode::Full, "bypassPermissions"),
    ] {
        let (directory, mut command) = setup();
        command.access_mode = access;
        let mut session = session(&command, directory.path());
        // The transport uses the first advertised mode as the initial selection.
        assert_eq!(session.modes[0]["id"], initial);
        assert!(session.select_mode("plan").is_err());
        let expected = if access == HarnessAccessMode::Auto {
            session.select_mode("default").expect("ask permissions");
            session.select_mode("auto").expect("restore auto mode");
            vec![json!("default"), json!("auto")]
        } else {
            assert!(session.select_mode("auto").is_err());
            Vec::new()
        };
        session.close().expect("close fixture session");
        let requests = std::fs::read_to_string(directory.path().join("claude-fixture.requests"))
            .expect("read fixture requests");
        let modes: Vec<Value> = requests
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("parse fixture request"))
            .filter(|frame| frame["request"]["subtype"] == "set_permission_mode")
            .map(|frame| frame["request"]["mode"].clone())
            .collect();
        assert_eq!(modes, expected);
    }
}

#[test]
fn access_modes_preserve_claude_model_auto_support() {
    for support in [None, Some(false), Some(true)] {
        let (directory, command) = setup();
        let field = support
            .map(|value| format!(",\"supportsAutoMode\":{value}"))
            .unwrap_or_default();
        let script = SCRIPT.replace(
            "\"supportsEffort\":true",
            &format!("\"supportsEffort\":true{field}"),
        );
        std::fs::write(directory.path().join("claude-fixture"), script)
            .expect("test operation should succeed");
        let mut session = session(&command, directory.path());
        let model: crate::protocol::Model = serde_json::from_value(session.models[0].clone())
            .expect("test operation should succeed");
        let modes = crate::agents::available_access_modes(BACKEND, Some(&model));
        assert_eq!(
            modes.contains(&HarnessAccessMode::Auto),
            support == Some(true)
        );
        assert!(modes.contains(&HarnessAccessMode::Sandboxed));
        assert!(modes.contains(&HarnessAccessMode::Full));
        session.close().expect("test operation should succeed");
    }
}
