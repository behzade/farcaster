use super::*;
use std::{error::Error, fs, os::unix::fs::PermissionsExt as _};
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn Error>>;

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "shell_environment_integration_tests.rs"]
mod integration;

#[test]
fn account_record_yields_its_absolute_login_shell() {
    assert_eq!(
        parse_account_login_shell(b"user:*:501:20::0:0:User:/Users/user:/opt/bin/fish\n"),
        Some(PathBuf::from("/opt/bin/fish"))
    );
    assert_eq!(parse_account_login_shell(b"malformed"), None);
}

#[test]
fn project_path_handoff_uses_the_captured_path() {
    let environment = with_project_path_handoff(vec![
        (OsString::from("PATH"), OsString::from("/captured/bin")),
        (
            OsString::from(PROJECT_PATH_HANDOFF),
            OsString::from("/stale/bin"),
        ),
        (OsString::from("HOME"), OsString::from("/home/user")),
    ]);

    assert_eq!(
        environment
            .iter()
            .filter(|(name, _)| name == PROJECT_PATH_HANDOFF)
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>(),
        vec![OsString::from("/captured/bin")],
    );
}

#[test]
fn parser_ignores_shell_output_outside_markers() -> TestResult {
    let output = [
        b"startup chatter\n".as_slice(),
        START_MARKER,
        b"PATH=/opt/tools:/usr/bin\0VALUE=left=right\0",
        END_MARKER,
        b"shutdown chatter\n",
    ]
    .concat();
    let environment = parse_environment(&output)?;
    assert_eq!(
        environment,
        vec![
            (
                OsString::from("PATH"),
                OsString::from("/opt/tools:/usr/bin")
            ),
            (OsString::from("VALUE"), OsString::from("left=right")),
        ]
    );
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn capture_runs_a_real_interactive_login_shell() -> TestResult {
    let temp = tempdir()?;
    let shell = temp.path().join("shell");
    let run_capture = r#"
test "$#" = "2"
exec /bin/sh
"#;
    let shell_script = format!(
        r#"#!/bin/sh
set -eu
test "$1" = "-l"
test "$2" = "-i"
test -t 0
test -t 1
export PATH=/login/bin:$PATH
export LOGIN_VALUE=loaded
export PROJECT_VALUE="$PWD"
export MULTILINE_VALUE='left
right'
printf 'prompt hook output before environment\n'
{run_capture}"#
    );
    fs::write(&shell, shell_script)?;
    let mut permissions = fs::metadata(&shell)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&shell, permissions)?;

    let environment = capture_login_shell_environment(&shell, temp.path())?;
    assert!(environment.iter().any(|(name, value)| {
        name == "PATH" && value.to_string_lossy().starts_with("/login/bin:")
    }));
    assert!(
        environment
            .iter()
            .any(|(name, value)| { name == "LOGIN_VALUE" && value == "loaded" })
    );
    assert!(environment.iter().any(|(name, value)| {
        name == "MULTILINE_VALUE" && value == &OsString::from("left\nright")
    }));
    let project = environment
        .iter()
        .find(|(name, _)| name == "PROJECT_VALUE")
        .map(|(_, value)| PathBuf::from(value))
        .ok_or("PROJECT_VALUE was not captured")?;
    assert_eq!(fs::canonicalize(project)?, fs::canonicalize(temp.path())?);
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn capture_includes_environment_loaded_by_first_interactive_prompt() -> TestResult {
    let temp = tempdir()?;
    let shell = temp.path().join("shell");
    let shell_script = r#"#!/bin/sh
set -eu
test "$1" = "-l"
test "$2" = "-i"
test -t 0
test -t 1
shift 2
if test "$#" -gt 0; then
    test "$1" = "-c"
    exec /bin/sh -c "$2"
fi
# Simulate environment loaded by fish_prompt, precmd, or PROMPT_COMMAND.
export FIRST_PROMPT_VALUE=loaded
exec /bin/sh
"#;
    fs::write(&shell, shell_script)?;
    let mut permissions = fs::metadata(&shell)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&shell, permissions)?;

    let environment = capture_login_shell_environment(&shell, temp.path())?;
    assert!(
        environment
            .iter()
            .any(|(name, value)| { name == "FIRST_PROMPT_VALUE" && value == "loaded" })
    );
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn capture_answers_a_query_over_the_real_pty_before_environment_capture() -> TestResult {
    let temp = tempdir()?;
    let shell = temp.path().join("querying shell");
    fs::write(
        &shell,
        r#"#!/bin/sh
set -eu
# Buffer the queued command, as fish does while waiting for terminal replies.
IFS= read -r capture
stty -echo -icanon min 0 time 10
printf '\033[0c'
reply=$(dd bs=1 count=5 2>/dev/null)
test "$reply" = "$(printf '\033[?0c')"
export HANDSHAKE_VALUE=answered
exec /bin/sh -c "$capture"
"#,
    )?;
    fs::set_permissions(&shell, fs::Permissions::from_mode(0o755))?;
    let environment = capture_login_shell_environment(&shell, temp.path())?;
    assert!(environment.contains(&("HANDSHAKE_VALUE".into(), "answered".into())));
    Ok(())
}

#[test]
fn terminal_answers_primary_attributes_across_read_boundaries() -> TestResult {
    // Only primary requests get replies, not prompts, replies, or optional queries.
    let output = b"prompt> \x1b[?0c\x1b[>0c\x1b[6n\x1b]11;?\x1b\\\x1b[0c\x1b[c\x1b[0c";
    for split in 0..=output.len() {
        let mut terminal = CaptureTerminal::default();
        let mut replies = Vec::new();
        terminal.respond(&output[..split], &mut replies)?;
        terminal.respond(&output[split..], &mut replies)?;
        assert_eq!(replies, b"\x1b[?0c\x1b[?0c\x1b[?0c", "split {split}");
    }
    Ok(())
}

#[test]
fn terminal_never_interprets_captured_values_as_queries() -> TestResult {
    let output = [
        b"\x1b[0c".as_slice(),
        START_MARKER,
        b"PATH=/bin\0VALUE=\x1b[0c\x1b[c\0",
        END_MARKER,
        b"\x1b[0c",
    ]
    .concat();
    for chunk_size in 1..=output.len() {
        let mut terminal = CaptureTerminal::default();
        let mut replies = Vec::new();
        for chunk in output.chunks(chunk_size) {
            terminal.respond(chunk, &mut replies)?;
        }
        assert_eq!(replies, b"\x1b[?0c", "chunk size {chunk_size}");
    }
    assert!(parse_environment(&output)?.contains(&("VALUE".into(), "\x1b[0c\x1b[c".into())));
    Ok(())
}

#[test]
fn parser_rejects_incomplete_or_pathless_output() {
    assert!(parse_environment(START_MARKER).is_err());
    assert!(parse_environment(&[START_MARKER, b"HOME=/home/user\0", END_MARKER].concat()).is_err());
}
