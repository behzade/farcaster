use super::*;
use crate::agents::HarnessAccessMode;
use std::{error::Error, fs};
use tempfile::tempdir;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn fake(case: &str) -> TestResult<(tempfile::TempDir, AgentLaunchConfig)> {
    let temp = tempdir()?;
    let script = temp.path().join("fake.sh");
    fs::write(
        &script,
        include_str!("../../../../../tests/fixtures/fake-pi.sh"),
    )?;
    let command = AgentLaunchConfig::test_script(&script, vec![case.into()]);
    Ok((temp, command))
}

#[test]
fn process_starts_directly_in_the_project_directory() -> TestResult {
    let (temp, command) = fake("project-directory")?;
    let mut rpc = PiRpcProcess::spawn(&command, temp.path(), None)?;
    let process_project = fs::read_to_string(temp.path().join("process-project"))?;
    assert_eq!(
        fs::canonicalize(process_project)?,
        fs::canonicalize(temp.path())?,
    );
    let mcp_config = serde_json::from_slice::<serde_json::Value>(&fs::read(
        temp.path().join("process-mcp-config"),
    )?)?;
    assert_eq!(
        mcp_config["mcpServers"]["farcaster"]["url"],
        "http://127.0.0.1:8765/mcp"
    );
    assert!(!temp.path().join(".mcp.json").exists());
    rpc.terminate()?;
    Ok(())
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
        assert!(!temp.path().join("process-mcp-config").exists());
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
        Some(Path::new("/dev/fd/9")),
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
        Some(Path::new("/dev/fd/9")),
    )?;
    let arguments = process.get_args().collect::<Vec<_>>();
    assert!(arguments.windows(2).any(|pair| pair == ["--mode", "rpc"]));
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["--mcp-config", "/dev/fd/9"])
    );
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
    let project = tempdir()?;
    let process = rpc_command(
        &AgentLaunchConfig::default(),
        project.path(),
        SessionLaunch::New,
        None,
    )?;
    let arguments = process.get_args().collect::<Vec<_>>();
    assert!(!arguments.iter().any(|argument| *argument == "--mcp-config"));
    assert!(
        !arguments
            .iter()
            .any(|argument| *argument == "--append-system-prompt")
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
            Some(Path::new("/dev/fd/9")),
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
        crate::agents::SessionOperation::SelectReasoning
    );
    rpc.terminate()?;
    Ok(())
}

#[test]
fn sandbox_adapter_detects_each_launch_and_blocks_failed_control() -> TestResult {
    use HarnessAccessMode::{Full, Sandboxed};
    for mode in [Sandboxed, Full] {
        let (temp, mut command) = fake("sandbox-ready")?;
        command.access_mode = mode;
        let mut process = PiRpcProcess::spawn(&command, temp.path(), None)?;
        assert_eq!(process.confirmed_sandbox_mode(), Some(mode));
        assert!(!temp.path().join("agent-prompts").exists());
        process.request_and_wait(SessionCommand::Prompt {
            mode: crate::protocol::PromptMode::Normal,
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
fn sandbox_control_needs_more_than_a_successful_rpc_response() -> TestResult {
    let (temp, command) = fake("quiet")?;
    let mut process = PiRpcProcess::spawn(&command, temp.path(), None)?;
    let result = process.confirm_control(
        serde_json::json!({"type":"get_state"}),
        Duration::from_millis(50),
        |_| None,
    );
    assert!(result.unwrap_err().contains("did not confirm"));
    Ok(())
}

#[test]
fn sandbox_mode_drift_revokes_confirmation_and_blocks_further_prompts() -> TestResult {
    let (temp, mut command) = fake("sandbox-ready")?;
    command.access_mode = HarnessAccessMode::Sandboxed;
    let mut process = PiRpcProcess::spawn(&command, temp.path(), None)?;
    let report = serde_json::json!({"version":1,"requestId":"external","files":"full","network":"full","success":true});
    let event = process.route(ReaderItem::Wire(Ok(PiWireMessage::ExtensionUi(
        crate::agents::extensions::ExtensionUiRequest::SetStatus {
            id: "mode-change".into(),
            key: "\u{1f}pi-gpui-sandbox-mode\u{1f}".into(),
            text: Some(report.to_string()),
        },
    ))));
    assert!(matches!(event, SessionEvent::Failure(_)));
    assert_eq!(process.confirmed_sandbox_mode(), None);
    assert!(
        process
            .send_request(SessionCommand::Prompt {
                mode: crate::protocol::PromptMode::Normal,
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
    let (temp, command) = fake("sandbox-ready")?;
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
    let (temp, command) = fake("sandbox-collision")?;
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
            access_mode: mode.unwrap_or(HarnessAccessMode::Auto),
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
        let Ok(crate::agents::SessionResponsePayload::LoadState(state)) = response.result else {
            panic!("state");
        };
        assert_eq!(state.message_count, 0);
        while let Some(event) = process.try_next() {
            assert!(!matches!(event, SessionEvent::Activity(ref activity)
                if matches!(activity.kind(), crate::agents::SessionActivityKind::AgentStarted)));
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
        matches!(rpc.try_next(), Some(SessionEvent::Activity(value)) if value.kind() == &crate::agents::SessionActivityKind::AgentStarted)
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
                Ok(crate::agents::SessionResponsePayload::LoadUsage(usage))
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
    let registry = crate::modules::agents::core::CallerRegistry::shared();
    let sender = registry.issue(
        temp.path(),
        crate::modules::agents::core::CallerProfile {
            backend: "pi".into(),
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
        mode: crate::protocol::PromptMode::Normal,
        message: "keep working".into(),
        images: Vec::new(),
    })?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut started = false;
    while Instant::now() < deadline {
        if matches!(
            rpc.try_next(),
            Some(SessionEvent::Activity(activity))
                if activity.kind() == &crate::agents::SessionActivityKind::AgentStarted
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
    let registry = crate::agents::CallerRegistry::shared();
    let parent = registry.issue(
        temp.path(),
        crate::modules::agents::core::CallerProfile {
            backend: "pi".into(),
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
    let registry = crate::agents::CallerRegistry::shared();
    let parent = registry.issue(
        temp.path(),
        crate::modules::agents::core::CallerProfile {
            backend: "pi".into(),
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
        Ok(crate::agents::SessionResponsePayload::LoadState(_))
    ));
    rpc.terminate()?;
    Ok(())
}
