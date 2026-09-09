use super::*;
use std::thread;
use std::time::{Duration, Instant};

const FIXTURES: &str =
    include_str!("../../../../../crates/claude-sdk-types/fixtures/protocol.json");

fn fixture(name: &str) -> Value {
    serde_json::from_str::<Vec<Value>>(FIXTURES)
        .unwrap()
        .into_iter()
        .find(|fixture| fixture["rust_type"] == name)
        .unwrap()["value"]
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
    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("claude-fixture");
    std::fs::write(&script, SCRIPT).unwrap();
    let mut result = fixture("SDKResultSuccess");
    result["result"] = json!("fixture ok");
    std::fs::write(script.with_extension("result"), format!("{result}\n")).unwrap();
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
    .unwrap();
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
    let process = Process::spawn(command, project, id, false, None, None, true).unwrap();
    attach(process, caller, id, command.access_mode).unwrap().0
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
    session.select_model(BACKEND, "fixture").unwrap();
    session.select_effort("high").unwrap();
    assert!(session.select_effort("max").is_err());
    assert!(session.select_mode("bypassPermissions").is_err());
    session.select_mode("plan").unwrap();
    session
        .send("hello".into(), WorkerSendMode::Prompt)
        .unwrap();
    session
        .send("second".into(), WorkerSendMode::Queue)
        .unwrap();
    for _ in 0..2 {
        let events = until(&mut session, |event| {
            matches!(event, WorkerEvent::NeedsInput(_))
        });
        let WorkerEvent::NeedsInput(input) = events.last().unwrap() else {
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
            .unwrap();
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
    session.close().unwrap();
    let requests =
        std::fs::read_to_string(directory.path().join("claude-fixture.requests")).unwrap();
    assert!(requests.contains("\"updatedInput\":{\"file_path\":905}"));
    assert!(requests.contains("\"effortLevel\":\"high\""));
}

#[test]
fn interrupt_drops_queue_and_process_exit_fails_once() {
    let (directory, command) = setup();
    let mut session = session(&command, directory.path());
    session.send("hold".into(), WorkerSendMode::Prompt).unwrap();
    session
        .send("not delivered".into(), WorkerSendMode::Queue)
        .unwrap();
    session.abort().unwrap();
    until(&mut session, |event| {
        matches!(event, WorkerEvent::Settled { .. })
    });
    assert!(session.queued.is_empty());
    session
        .send("crash".into(), WorkerSendMode::Prompt)
        .unwrap();
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
    for access in [HarnessAccessMode::Sandboxed, HarnessAccessMode::Full] {
        let mut command = std::process::Command::new("claude");
        super::super::process::configure(&mut command, access, "id", true, None, false);
        let args = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect::<Vec<_>>();
        assert!(args.contains(&"--resume=id"));
        assert!(args.contains(&"--no-session-persistence"));
        assert!(args.contains(&"--permission-prompt-tool"));
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
    .unwrap();
    let value = serde_json::to_value(message).unwrap();
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
    let metadata = load_configuration(&command, directory.path()).unwrap();
    assert_eq!(metadata.models[0]["id"], "fixture");
    let args = std::fs::read_to_string(directory.path().join("claude-fixture.args")).unwrap();
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
    let (mut worker, locator, _) = spawn_main(&command, &launch).unwrap();
    assert_eq!(locator, id);
    worker.close().unwrap();
    let args = std::fs::read_to_string(directory.path().join("claude-fixture.args")).unwrap();
    assert!(args.lines().any(|arg| arg == format!("--resume={id}")));
    assert!(!args.lines().any(|arg| arg == "--no-session-persistence"));
    if super::super::super::farcaster_mcp::enabled() {
        assert!(args.lines().any(|arg| arg == "--mcp-config"));
        assert!(args.contains("farcaster-caller"));
    }
    let requests =
        std::fs::read_to_string(directory.path().join("claude-fixture.requests")).unwrap();
    assert!(!requests.contains("\"type\":\"user\""));
}

#[test]
fn cancelling_an_approval_denies_without_changing_tool_input() {
    let (directory, command) = setup();
    let mut session = session(&command, directory.path());
    session
        .send("hello".into(), WorkerSendMode::Prompt)
        .unwrap();
    let events = until(&mut session, |event| {
        matches!(event, WorkerEvent::NeedsInput(_))
    });
    let WorkerEvent::NeedsInput(input) = events.last().unwrap() else {
        unreachable!()
    };
    session
        .respond(WorkerInputResponse {
            id: input.id.clone(),
            value: Some("Allow".into()),
            cancel: true,
        })
        .unwrap();
    until(&mut session, |event| {
        matches!(event, WorkerEvent::Settled { .. } | WorkerEvent::Failed(_))
    });
    session.close().unwrap();
    let requests =
        std::fs::read_to_string(directory.path().join("claude-fixture.requests")).unwrap();
    assert!(requests.contains("\"behavior\":\"deny\""));
    assert!(!requests.contains("\"behavior\":\"allow\""));
}
