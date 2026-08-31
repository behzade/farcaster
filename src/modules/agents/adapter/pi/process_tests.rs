use super::*;
use crate::agents::{FileAccessMode, NetworkAccessMode, PermissionLevel};
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
fn fork_process_passes_the_source_session_to_pi() -> TestResult {
    let project = tempdir()?;
    let source = Path::new("/sessions/source session.jsonl");
    let process = rpc_command(
        &AgentLaunchConfig {
            sandbox: crate::access::test_sandbox_bypass(),
            ..AgentLaunchConfig::default()
        },
        project.path(),
        SessionLaunch::Fork(source),
        Path::new("/dev/fd/9"),
    )?;
    let arguments = process.command.get_args().collect::<Vec<_>>();
    assert!(arguments.windows(2).any(|pair| pair == ["--mode", "rpc"]));
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["--mcp-config", "/dev/fd/9"])
    );
    assert!(arguments.windows(2).any(|pair| {
        pair == [
            std::ffi::OsStr::new("--append-system-prompt"),
            std::ffi::OsStr::new(crate::modules::agents::adapter::farcaster_mcp::INSTRUCTIONS),
        ]
    }));
    assert_eq!(
        arguments.get(arguments.len().saturating_sub(2)..),
        Some([std::ffi::OsStr::new("--fork"), source.as_os_str()].as_slice())
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
fn pi_disables_its_inner_sandbox_inside_farcaster() -> TestResult {
    let project = tempdir()?;
    let nono = project.path().join("nono");
    fs::write(&nono, b"#!/bin/sh\nexit 0\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&nono, fs::Permissions::from_mode(0o700))?;
    }
    let pi = project.path().join("pi");
    fs::write(&pi, b"#!/bin/sh\nexit 0\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&pi, fs::Permissions::from_mode(0o700))?;
    }
    let command = AgentLaunchConfig {
        program: pi,
        permission_level: PermissionLevel {
            files: FileAccessMode::ReadOnly,
            network: NetworkAccessMode::Sandboxed,
        },
        sandbox: crate::access::SandboxRuntime::fixed(nono.clone()),
        ..AgentLaunchConfig::default()
    };
    let prepared = rpc_command(
        &command,
        project.path(),
        SessionLaunch::New,
        Path::new("/dev/fd/9"),
    )?;
    assert_eq!(prepared.command.get_program(), nono.as_os_str());
    let arguments = prepared.command.get_args().collect::<Vec<_>>();
    assert!(!arguments.iter().any(|argument| {
        matches!(
            argument.to_str(),
            Some("--sandbox-files" | "--sandbox-network")
        )
    }));
    assert!(prepared.command.get_envs().any(|(name, value)| {
        name == "PI_NONO_DISABLED" && value == Some(std::ffi::OsStr::new("1"))
    }));
    Ok(())
}

#[test]
fn request_and_wait_confirms_configuration_before_returning() -> TestResult {
    let (temp, command) = fake("normal")?;
    let mut rpc = PiRpcProcess::spawn(&command, temp.path(), None)?;
    let response = rpc.request_and_wait(SessionCommand::SelectReasoning {
        level: "medium".into(),
    })?;
    assert_eq!(response.command, "set_thinking_level");
    rpc.terminate()?;
    Ok(())
}

#[test]
fn handshake_routes_async_event_and_correlates_unique_ids() -> TestResult {
    let (temp, command) = fake("normal")?;
    let mut rpc = PiRpcProcess::spawn(&command, temp.path(), None)?;
    assert!(
        matches!(rpc.try_next(), Some(SessionEvent::Activity(value)) if value["type"] == "agent_start")
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
            context_shape |= response.command == "get_session_stats"
                && response
                    .data
                    .pointer("/contextUsage/tokens")
                    .and_then(serde_json::Value::as_u64)
                    == Some(4096)
                && response
                    .data
                    .pointer("/contextUsage/contextWindow")
                    .and_then(serde_json::Value::as_u64)
                    == Some(8192)
                && response
                    .data
                    .pointer("/contextUsage/percent")
                    .and_then(serde_json::Value::as_u64)
                    == Some(50);
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(responses, 3);
    assert!(context_shape);
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
