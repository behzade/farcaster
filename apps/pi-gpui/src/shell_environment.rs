//! macOS login-shell environment import for app-bundle launches.

use std::{
    ffi::OsString,
    io::Write as _,
    os::unix::ffi::OsStringExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const IMPORT_REQUESTED: &str = "PI_GUI_IMPORT_SHELL_ENV";
const START_MARKER: &[u8] = b"\x1ePI_GPUI_ENV_START\x1f\0";
const END_MARKER: &[u8] = b"\x1ePI_GPUI_ENV_END\x1f\0";
const CAPTURE_COMMAND: &str = "/bin/stty -echo -opost; /usr/bin/printf '\\036PI_GPUI_ENV_START\\037\\0'; /usr/bin/env -0; /usr/bin/printf '\\036PI_GPUI_ENV_END\\037\\0'; exit\n";

type Environment = Vec<(OsString, OsString)>;

pub(crate) fn project_shell_environment(project: &Path) -> Result<Option<Environment>, String> {
    if std::env::var(IMPORT_REQUESTED).as_deref() != Ok("1") {
        return Ok(None);
    }
    capture_login_shell_environment(&default_login_shell(), project).map(Some)
}

fn default_login_shell() -> PathBuf {
    account_login_shell()
        .or_else(|| std::env::var_os("SHELL").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/bin/zsh"))
}

fn account_login_shell() -> Option<PathBuf> {
    let output = Command::new("/usr/bin/id").arg("-P").output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_account_login_shell(&output.stdout)
}

fn parse_account_login_shell(output: &[u8]) -> Option<PathBuf> {
    let mut shell = output.rsplit(|byte| *byte == b':').next()?;
    while matches!(shell.last(), Some(b'\n' | b'\r')) {
        shell = &shell[..shell.len() - 1];
    }
    if shell.first() != Some(&b'/') {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(shell.to_vec())))
}

fn capture_login_shell_environment(shell: &Path, project: &Path) -> Result<Environment, String> {
    let mut child = Command::new("/usr/bin/script")
        .args(["-q", "/dev/null"])
        .arg(shell)
        .args(["-l", "-i"])
        .current_dir(project)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start login shell {} in terminal: {error}", shell.display()))?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| "login shell terminal did not expose input".to_owned())?;
    input
        .write_all(CAPTURE_COMMAND.as_bytes())
        .and_then(|()| input.flush())
        .map_err(|error| format!("request login shell environment: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("wait for login shell {}: {error}", shell.display()))?;
    drop(input);
    if !output.status.success() {
        return Err(format!(
            "login shell {} exited with {}",
            shell.display(),
            output.status
        ));
    }
    parse_environment(&output.stdout)
}

fn parse_environment(output: &[u8]) -> Result<Environment, String> {
    let start = find(output, START_MARKER)
        .map(|index| index + START_MARKER.len())
        .ok_or_else(|| {
            "login shell output did not contain the environment start marker".to_owned()
        })?;
    let end = find(&output[start..], END_MARKER)
        .map(|index| start + index)
        .ok_or_else(|| {
            "login shell output did not contain the environment end marker".to_owned()
        })?;

    let mut environment = Vec::new();
    for record in output[start..end].split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        let separator = record
            .iter()
            .position(|byte| *byte == b'=')
            .ok_or_else(|| "login shell returned a malformed environment entry".to_owned())?;
        if separator == 0 {
            return Err("login shell returned an empty environment name".to_owned());
        }
        environment.push((
            OsString::from_vec(record[..separator].to_vec()),
            OsString::from_vec(record[separator + 1..].to_vec()),
        ));
    }
    if !environment.iter().any(|(name, _)| name == "PATH") {
        return Err("login shell environment did not contain PATH".to_owned());
    }
    Ok(environment)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
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

    #[cfg(target_os = "macos")]
    #[test]
    fn capture_runs_a_real_interactive_login_shell() -> TestResult {
        let temp = tempdir()?;
        let shell = temp.path().join("shell");
        fs::write(
            &shell,
            r#"#!/bin/sh
set -eu
test "$1" = "-l"
test "$2" = "-i"
test "$#" = "2"
test -t 0
test -t 1
export PATH=/login/bin:/usr/bin
export LOGIN_VALUE=loaded
export PROJECT_VALUE="$(/bin/pwd)"
export MULTILINE_VALUE='left
right'
printf 'prompt hook output before environment\n'
exec /bin/sh
"#,
        )?;
        let mut permissions = fs::metadata(&shell)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&shell, permissions)?;

        let environment = capture_login_shell_environment(&shell, temp.path())?;
        assert!(
            environment
                .iter()
                .any(|(name, value)| { name == "PATH" && value == "/login/bin:/usr/bin" })
        );
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

    #[test]
    fn parser_rejects_incomplete_or_pathless_output() {
        assert!(parse_environment(START_MARKER).is_err());
        assert!(
            parse_environment(&[START_MARKER, b"HOME=/home/user\0", END_MARKER].concat()).is_err()
        );
    }
}
