use super::*;
use std::{error::Error, fs, os::unix::fs::PermissionsExt as _};
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn Error>>;

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

#[cfg(target_os = "macos")]
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

#[test]
fn parser_rejects_incomplete_or_pathless_output() {
    assert!(parse_environment(START_MARKER).is_err());
    assert!(parse_environment(&[START_MARKER, b"HOME=/home/user\0", END_MARKER].concat()).is_err());
}
