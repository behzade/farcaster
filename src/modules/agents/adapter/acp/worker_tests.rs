use super::*;
use std::io::Write as _;

#[cfg(unix)]
#[test]
fn external_acp_processes_apply_access_prompt_cancel_and_resume() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};
    const SCRIPT: &str = r#"#!/bin/sh
reply() { printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$1"; }
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$0.requests"
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([^,}]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) reply '{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"close":{}}},"authMethods":[{"id":"oauth-personal","name":"Google account"}]}' ;;
    *'"method":"authenticate"'*) reply '{}' ;;
    *'"method":"session/new"'*|*'"method":"session/load"'*|*'"method":"session/resume"'*)
      reply '{"sessionId":"one","configOptions":[{"id":"model","category":"model","currentValue":"base","options":[{"value":"base","name":"Base"}]},{"id":"mode","category":"mode","currentValue":"default","options":[{"value":"default"},{"value":"bypassPermissions"},{"value":"yolo"}]}]}' ;;
    *'"method":"session/set_mode"'*|*'"method":"session/set_config_option"'*) reply '{}' ;;
    *'"method":"session/prompt"'*)
      prompt_id=$id
      case "$line" in
        *'hold'*) ;;
        *) printf '%s\n' '{"jsonrpc":"2.0","id":"approval","method":"session/request_permission","params":{"sessionId":"one","toolCall":{"toolCallId":"tool","title":"Read fixture","kind":"read","status":"pending"},"options":[{"optionId":"allow","name":"Allow","kind":"allow_once"},{"optionId":"deny","name":"Decline","kind":"reject_once"}]}}' ;;
      esac ;;
    *'"id":"approval"'*)
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"one","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"fixture ok"}}}}'
      id=$prompt_id; reply '{"stopReason":"end_turn"}' ;;
    *'"method":"session/cancel"'*) id=$prompt_id; reply '{"stopReason":"cancelled"}' ;;
    *'"method":"session/close"'*) reply '{}'; exit 0 ;;
    *) exit 2 ;;
  esac
done
"#;
    fn settle(session: &mut AcpWorkerSession) -> (String, usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut approvals = 0;
        while Instant::now() < deadline {
            match session.poll() {
                Some(WorkerEvent::NeedsInput(input)) => {
                    approvals += 1;
                    session
                        .respond(WorkerInputResponse {
                            id: input.id,
                            value: Some("Allow".into()),
                            cancel: false,
                        })
                        .expect("test operation should succeed");
                }
                Some(WorkerEvent::Settled { output }) => return (output, approvals),
                Some(WorkerEvent::Failed(error)) => panic!("{error}"),
                _ => thread::sleep(Duration::from_millis(5)),
            }
        }
        panic!("fixture did not settle");
    }
    for profile in [&super::super::super::antigravity::PROFILE] {
        for access_mode in [HarnessAccessMode::Sandboxed, HarnessAccessMode::Full] {
            let project = tempfile::tempdir().expect("test operation should succeed");
            let executable = project.path().join("agent");
            std::fs::write(&executable, SCRIPT).expect("test operation should succeed");
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
                .expect("test operation should succeed");
            std::fs::write(project.path().join("localharness_external"), "fixture")
                .expect("test operation should succeed");
            let command = AgentLaunchConfig {
                program: executable.clone(),
                prefix_args: vec![],
                access_mode,
                app_proxy: None,
                session_locator_root: None,
            };
            let (mut session, metadata, _) =
                spawn_session(&command, profile, project.path(), None, None, None)
                    .expect("test operation should succeed");
            assert_eq!(
                metadata.modes[0]["id"],
                profile
                    .permission_mode(access_mode)
                    .expect("test operation should succeed")
            );
            session
                .send("hello".into(), WorkerSendMode::Prompt)
                .expect("test operation should succeed");
            assert_eq!(settle(&mut session), ("fixture ok".into(), 1));
            session
                .send("hold".into(), WorkerSendMode::Prompt)
                .expect("test operation should succeed");
            session.abort().expect("test operation should succeed");
            assert_eq!(settle(&mut session).1, 0);
            session.close().expect("test operation should succeed");
            assert!(
                session
                    .child
                    .try_wait()
                    .expect("test operation should succeed")
                    .is_some()
            );
            let (mut resumed, _, _) =
                spawn_session(&command, profile, project.path(), Some("one"), None, None)
                    .expect("test operation should succeed");
            resumed.close().expect("test operation should succeed");
            let requests = std::fs::read_to_string(executable.with_extension("requests"))
                .expect("test operation should succeed");
            assert!(requests.contains(profile.resume_method));
            assert_eq!(
                requests.contains("authenticate"),
                profile.auth_method.is_some()
            );
            assert!(
                requests.contains(
                    profile
                        .permission_mode(access_mode)
                        .expect("test operation should succeed")
                )
            );
        }
    }
}

const PROFILE: AcpProfile = AcpProfile {
    backend: "test-acp",
    name: "Test ACP",
    command: "test-acp",
    path_environment: "FARCASTER_TEST_ACP_PATH",
    arguments: &["acp"],
    auth_method: None,
    force_argument: Some("--force"),
    resume_method: "session/load",
    permission_modes: None,
};

#[cfg(unix)]
fn inert_session() -> AcpWorkerSession {
    AcpWorkerSession {
        profile: PROFILE.clone(),
        child: std::process::Command::new("true")
            .spawn()
            .expect("test operation should succeed"),
        connection: AcpConnection::new(
            futures::io::Cursor::new(Vec::<u8>::new()),
            futures::io::Cursor::new(Vec::<u8>::new()),
            None,
        )
        .expect("test operation should succeed"),
        session_id: "one".into(),
        current_prompt: Some(AcpRequestId::Number(1)),
        next_prompt_ack: None,
        prompt_requests: HashMap::new(),
        prompt_acks: VecDeque::new(),
        pending_steers: HashMap::new(),
        queued_prompts: VecDeque::new(),
        output: String::new(),
        thought_started: false,
        pending_inputs: HashMap::new(),
        tool_states: HashMap::new(),
        peer_messages: VecDeque::new(),
        events: VecDeque::new(),
        config_ids: ConfigIds::default(),
        features: AcpFeatures {
            steering: false,
            close: false,
        },
        caller_identity: None,
    }
}

#[cfg(unix)]
#[test]
fn model_and_service_tier_are_sent_independently() {
    use std::io::{BufRead as _, Write as _};
    use std::os::unix::net::UnixStream;
    let (client, peer) = UnixStream::pair().expect("test operation should succeed");
    peer.set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .expect("test operation should succeed");
    let peer = thread::spawn(move || {
        let mut peer = std::io::BufReader::new(peer);
        for (config, value) in [
            ("model", "base"),
            ("context", "1m"),
            ("fast", "true"),
            ("fast", "false"),
        ] {
            let mut line = String::new();
            peer.read_line(&mut line)
                .expect("test operation should succeed");
            let request: Value =
                serde_json::from_str(&line).expect("test operation should succeed");
            assert_eq!(request["method"], "session/set_config_option");
            assert_eq!(request["params"]["configId"], config);
            assert_eq!(request["params"]["value"], value);
            let response = json!({"jsonrpc":"2.0","id":request["id"],"result":{"configOptions":[
                {"id":"model","category":"model","currentValue":"base","options":[{"value":"base"}]},
                {"id":"context","category":"model_config","currentValue":if config == "model" {"272k"} else {"1m"}},
                {"id":"fast","category":"model_config","currentValue":if config == "fast" {value} else {"false"},"options":[{"value":"false"},{"value":"true"}]},
                {"id":"effort","category":"thought_level","currentValue":"high","options":[{"value":"high"}]}
            ]}});
            writeln!(peer.get_mut(), "{response}").expect("test operation should succeed");
            peer.get_mut()
                .flush()
                .expect("test operation should succeed");
        }
    });
    let mut session = inert_session();
    session.profile = super::super::super::cursor::PROFILE;
    session.connection = AcpConnection::new(
        blocking::Unblock::new(client.try_clone().expect("test operation should succeed")),
        blocking::Unblock::new(client),
        None,
    )
    .expect("test operation should succeed");
    session.config_ids.model = Some("model".into());
    session.config_ids.service_tier = Some("fast".into());
    session.config_ids.selected_service_tier = Some("priority".into());
    session.config_ids.catalog = vec![json!({"value":"base","configOptions":[
        {"id":"context","category":"model_config","options":[{"value":"272k"},{"value":"1m"}]},
        {"id":"fast","category":"model_config","options":[{"value":"false"},{"value":"true"}]}
    ]})];
    session.config_ids.selections.insert(
        "base[context=1m]".into(),
        super::super::configuration::ModelSelection {
            model: "base".into(),
            parameters: vec![("context".into(), "1m".into())],
        },
    );
    session
        .select_model("cursor-cli", "base[context=1m]")
        .expect("test operation should succeed");
    assert_eq!(
        session.config_ids.selected_service_tier.as_deref(),
        Some("priority")
    );
    let model = session.config_ids.selected_model.clone();
    session
        .select_service_tier("standard")
        .expect("test operation should succeed");
    assert_eq!(session.config_ids.selected_model, model);
    assert_eq!(model.as_deref(), Some("base[context=1m]"));
    assert_eq!(
        session.config_ids.selected_service_tier.as_deref(),
        Some("standard")
    );
    assert!(session.events.iter().any(|event| matches!(event,
        WorkerEvent::Activity(WorkerActivity::ServiceTierChanged {selected:Some(tier),options})
            if tier == "priority" && options == &["standard", "priority"])));
    peer.join().expect("test operation should succeed");
}

#[cfg(unix)]
#[test]
fn metadata_and_plan_updates_stay_neutral_and_replace_prior_plan() {
    let mut session = inert_session();
    assert!(
        matches!(session.update(json!({"sessionId":"one","update":{"sessionUpdate":"current_mode_update","currentModeId":"ask"}})),
        Some(WorkerEvent::Activity(WorkerActivity::ModeChanged(mode))) if mode == "ask")
    );
    assert!(
        matches!(session.update(json!({"sessionId":"one","update":{"sessionUpdate":"session_info_update","title":"Named"}})),
        Some(WorkerEvent::Activity(WorkerActivity::TitleChanged(title))) if title == "Named")
    );
    assert!(session.update(json!({"sessionId":"other","update":{"sessionUpdate":"session_info_update","title":"Wrong"}})).is_none());
    for (index, status) in ["pending", "completed"].into_iter().enumerate() {
        let event = session.update(json!({"sessionId":"one","update":{"sessionUpdate":"plan","entries":[{"content":"Check","status":status,"priority":"high"}]}})).expect("test operation should succeed");
        if index == 0 {
            assert!(matches!(
                event,
                WorkerEvent::Activity(WorkerActivity::ToolStarted { .. })
            ));
        } else {
            assert!(matches!(
                event,
                WorkerEvent::Activity(WorkerActivity::ToolMetadataChanged { .. })
            ));
        }
        assert!(
            matches!(session.events.pop_front(), Some(WorkerEvent::Activity(WorkerActivity::ToolFinished {result,..})) if result.to_string().contains(status))
        );
        assert!(session.events.is_empty());
    }
}

#[test]
fn full_access_uses_the_profile_escape_hatch() {
    let mut command = std::process::Command::new("agent");
    configure_command(&mut command, &PROFILE, HarnessAccessMode::Full)
        .expect("test operation should succeed");
    assert_eq!(
        command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        ["--force", "acp"]
    );
}

#[test]
#[ignore = "requires signed-in Cursor and network; creates a scratch session"]
fn live_cursor_configuration_and_listing() {
    let project = tempfile::tempdir().expect("test operation should succeed");
    let command = AgentLaunchConfig {
        program: "agent".into(),
        prefix_args: Vec::new(),
        access_mode: HarnessAccessMode::Auto,
        app_proxy: None,
        session_locator_root: None,
    };
    let profile = &super::super::super::cursor::PROFILE;
    let (mut session, metadata, _) =
        spawn_session(&command, profile, project.path(), None, None, None)
            .expect("test operation should succeed");
    assert!(!metadata.models.is_empty());
    assert!(
        metadata
            .models
            .iter()
            .all(|model| model["id"].as_str().is_some_and(|id| !id.contains("fast=")))
    );
    assert!(
        metadata
            .models
            .iter()
            .any(|model| model["contextWindow"].as_u64() == Some(1_000_000))
    );
    assert!(metadata.models.iter().any(|model| {
        model["efforts"]
            .as_array()
            .is_some_and(|efforts| !efforts.is_empty())
    }));
    let original_mode = metadata
        .modes
        .first()
        .expect("test operation should succeed")["id"]
        .as_str()
        .expect("test operation should succeed")
        .to_owned();
    if let Some(tier) = &metadata.service_tier {
        let original_model = session.config_ids.selected_model.clone();
        session
            .select_service_tier(tier)
            .expect("test operation should succeed");
        assert_eq!(session.config_ids.selected_model, original_model);
        assert_eq!(
            session.config_ids.selected_service_tier.as_ref(),
            Some(tier)
        );
        writeln!(
            std::io::stderr().lock(),
            "Service tier {tier} confirmed without changing model identity"
        )
        .expect("write test diagnostics");
    }
    session
        .select_mode("ask")
        .expect("test operation should succeed");
    assert!(session.events.iter().any(|event| matches!(event,
        WorkerEvent::Activity(WorkerActivity::ModeChanged(mode)) if mode == "ask")));
    assert!(session.events.iter().any(|event| matches!(
        event,
        WorkerEvent::Activity(WorkerActivity::ConfigurationChanged {
            selected_model: Some(_),
            ..
        })
    )));
    session
        .select_mode(&original_mode)
        .expect("test operation should succeed");
    session.close().expect("test operation should succeed");
    assert!(
        session
            .child
            .try_wait()
            .expect("test operation should succeed")
            .is_some()
    );
    let sessions =
        super::super::catalog::list_sessions(profile).expect("test operation should succeed");
    assert!(
        sessions
            .iter()
            .all(|entry| entry["sessionId"].is_string() && entry["cwd"].is_string())
    );
    writeln!(std::io::stderr().lock(),
        "Live configuration: {} model choices; service tier excluded from model IDs; context/effort present; mode response refreshed; listing returned {} sessions",
        metadata.models.len(),
        sessions.len()
    ).expect("write test diagnostics");
}

/// Uses the installed, signed-in Cursor CLI and makes real model requests.
#[test]
#[ignore = "requires Cursor login and network; consumes model usage"]
fn live_cursor_session_round_trip() {
    use std::time::{Duration, Instant};

    fn settle(session: &mut AcpWorkerSession) -> String {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            match session.poll() {
                Some(WorkerEvent::Settled { output }) => return output,
                Some(WorkerEvent::Activity(WorkerActivity::CommandsChanged { commands })) => {
                    writeln!(
                        std::io::stderr().lock(),
                        "Live command update: {} commands",
                        commands.len()
                    )
                    .expect("write test diagnostics");
                }
                Some(WorkerEvent::Failed(error)) => panic!("Cursor failed: {error}"),
                Some(WorkerEvent::NeedsInput(input)) => {
                    session
                        .respond(WorkerInputResponse {
                            id: input.id,
                            value: None,
                            cancel: true,
                        })
                        .expect("test operation should succeed");
                    panic!("no-tool prompt unexpectedly requested permission");
                }
                _ => thread::sleep(Duration::from_millis(20)),
            }
        }
        panic!("Cursor did not settle within 60 seconds");
    }

    fn permission_turn(session: &mut AcpWorkerSession, action: &str) {
        session.send(
            "Use the shell tool exactly once to run `printf FARCASTER_PERMISSION_CHECK`. Do not read or change files or run any other command. If denied, do not retry; just reply DENIED.".into(),
            WorkerSendMode::Prompt,
        ).expect("test operation should succeed");
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut permissions = 0;
        let mut approvals = 0;
        while Instant::now() < deadline {
            match session.poll() {
                Some(WorkerEvent::NeedsInput(input)) => {
                    permissions += 1;
                    if action == "cancel" {
                        session.abort().expect("test operation should succeed");
                    } else {
                        // Only approve the harmless command named by this test.
                        let approved = action == "allow"
                            && input.prompt.contains("printf FARCASTER_PERMISSION_CHECK");
                        approvals += usize::from(approved);
                        session
                            .respond(WorkerInputResponse {
                                id: input.id,
                                value: Some(if approved { "Allow" } else { "Decline" }.into()),
                                cancel: false,
                            })
                            .expect("test operation should succeed");
                    }
                }
                Some(WorkerEvent::Settled { .. }) => {
                    writeln!(std::io::stderr().lock(),
                        "Permission phase {action}: {permissions} requests, {approvals} approved; settled"
                    ).expect("write test diagnostics");
                    assert!(session.pending_inputs.is_empty());
                    assert!(session.current_prompt.is_none());
                    return;
                }
                Some(WorkerEvent::Failed(error)) => panic!("permission phase {action}: {error}"),
                _ => thread::sleep(Duration::from_millis(20)),
            }
        }
        panic!("permission phase {action} did not settle");
    }

    let project = tempfile::tempdir().expect("test operation should succeed");
    let command = AgentLaunchConfig {
        program: "agent".into(),
        prefix_args: Vec::new(),
        access_mode: HarnessAccessMode::Auto,
        app_proxy: None,
        session_locator_root: None,
    };
    let profile = &super::super::super::cursor::PROFILE;
    let (mut session, metadata, history) = spawn_session(
        &command,
        profile,
        project.path(),
        None,
        None,
        Some(thread::current()),
    )
    .expect("create live Cursor session");
    assert!(history.is_none());
    writeln!(
        std::io::stderr().lock(),
        "Cursor session created; commands: {}, models: {}",
        metadata.commands.len(),
        metadata.models.len()
    )
    .expect("write test diagnostics");
    session
        .send(
            "Do not call tools or access files. Reply with exactly FARCASTER_ACP_LIVE_OK.".into(),
            WorkerSendMode::Queue,
        )
        .expect("test operation should succeed");
    assert!(settle(&mut session).contains("FARCASTER_ACP_LIVE_OK"));
    let locator = session.session_id.clone();
    session.close().expect("test operation should succeed");
    assert!(
        session
            .child
            .try_wait()
            .expect("test operation should succeed")
            .is_some()
    );
    writeln!(
        std::io::stderr().lock(),
        "Prompt settled and original process reaped"
    )
    .expect("write test diagnostics");

    let (mut resumed, _, history) = spawn_session(
        &command,
        profile,
        project.path(),
        Some(&locator),
        None,
        Some(thread::current()),
    )
    .expect("resume live Cursor session");
    let history = history.expect("resume history");
    assert_eq!(
        history.messages.len(),
        2,
        "expected one user and one assistant message"
    );
    writeln!(
        std::io::stderr().lock(),
        "Resume loaded {} history messages",
        history.messages.len()
    )
    .expect("write test diagnostics");
    resumed
        .send(
            "Do not call tools. Reply with exactly FARCASTER_ACP_RESUMED_OK.".into(),
            WorkerSendMode::Queue,
        )
        .expect("test operation should succeed");
    let output = settle(&mut resumed);
    assert!(
        output.contains("FARCASTER_ACP_RESUMED_OK"),
        "resumed output: {output:?}"
    );
    for action in ["allow", "deny", "cancel"] {
        permission_turn(&mut resumed, action);
    }
    // Cancel independently of whether Cursor asks permission for printf.
    resumed
        .send(
            "Do not use tools. Count from one to one hundred.".into(),
            WorkerSendMode::Prompt,
        )
        .expect("test operation should succeed");
    resumed.abort().expect("test operation should succeed");
    let _ = settle(&mut resumed);
    assert!(resumed.current_prompt.is_none());
    writeln!(std::io::stderr().lock(), "Immediate cancellation settled")
        .expect("write test diagnostics");
    resumed.close().expect("test operation should succeed");
    assert!(
        resumed
            .child
            .try_wait()
            .expect("test operation should succeed")
            .is_some()
    );
    writeln!(
        std::io::stderr().lock(),
        "Resumed prompt settled and process reaped"
    )
    .expect("write test diagnostics");
}

#[test]
fn acp_prompt_ack_waits_for_its_response_and_rejects_errors() {
    for (reply, accepted) in [
        (
            AcpInbound::Response {
                id: AcpRequestId::Number(1),
                result: json!({"stopReason":"end_turn"}),
            },
            true,
        ),
        (
            AcpInbound::Error {
                id: AcpRequestId::Number(1),
                message: "rejected".into(),
            },
            false,
        ),
        (
            AcpInbound::Response {
                id: AcpRequestId::Number(1),
                result: json!({}),
            },
            false,
        ),
    ] {
        let mut session = inert_session();
        session
            .prompt_requests
            .insert(AcpRequestId::Number(1), "submission".into());
        assert!(session.poll_prompt_ack().is_none());
        session.connection.restore_queued(VecDeque::from([reply]));
        session.poll();
        let (id, result) = session.poll_prompt_ack().expect("correlated reply");
        assert_eq!(id, "submission");
        assert_eq!(result.is_ok(), accepted);
    }
}

#[test]
fn acp_in_memory_queue_is_not_an_acknowledgement() {
    let mut session = inert_session();
    assert!(
        !session
            .submit_prompt(
                "queued".into(),
                "work".into(),
                WorkerSendMode::Queue,
                Vec::new()
            )
            .unwrap()
    );
    assert!(session.poll_prompt_ack().is_none());
    assert_eq!(
        session.queued_prompts.back().unwrap().3.as_deref(),
        Some("queued")
    );
}
