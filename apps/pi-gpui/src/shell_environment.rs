//! macOS login-shell environment import for app-bundle launches.

use std::{
    ffi::OsString,
    os::unix::ffi::OsStringExt as _,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

const IMPORT_REQUESTED: &str = "PI_GUI_IMPORT_SHELL_ENV";
const START_MARKER: &[u8] = b"\x1ePI_GPUI_ENV_START\x1f\0";
const END_MARKER: &[u8] = b"\x1ePI_GPUI_ENV_END\x1f\0";
const CAPTURE_COMMAND: &str = "/usr/bin/printf '\\036PI_GPUI_ENV_START\\037\\0'; /usr/bin/env -0; /usr/bin/printf '\\036PI_GPUI_ENV_END\\037\\0'";

type Environment = Vec<(OsString, OsString)>;

pub(crate) fn login_shell_environment() -> Option<&'static [(OsString, OsString)]> {
    if std::env::var(IMPORT_REQUESTED).as_deref() != Ok("1") {
        return None;
    }

    static ENVIRONMENT: OnceLock<Option<Environment>> = OnceLock::new();
    ENVIRONMENT
        .get_or_init(|| capture_login_shell_environment(&default_login_shell()).ok())
        .as_deref()
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

fn capture_login_shell_environment(shell: &Path) -> Result<Environment, String> {
    let output = Command::new(shell)
        .args(["-l", "-i", "-c", CAPTURE_COMMAND])
        .output()
        .map_err(|error| format!("start login shell {}: {error}", shell.display()))?;
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

    #[test]
    fn capture_runs_an_interactive_login_shell() -> TestResult {
        let temp = tempdir()?;
        let shell = temp.path().join("shell");
        fs::write(
            &shell,
            r#"#!/bin/sh
set -eu
test "$1" = "-l"
test "$2" = "-i"
test "$3" = "-c"
printf 'profile output before environment\n'
PATH=/login/bin:/usr/bin LOGIN_VALUE=loaded /bin/sh -c "$4"
"#,
        )?;
        let mut permissions = fs::metadata(&shell)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&shell, permissions)?;

        let environment = capture_login_shell_environment(&shell)?;
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
