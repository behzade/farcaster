use std::io::Write as _;

use std::{
    collections::HashMap,
    ffi::OsString,
    os::unix::ffi::OsStringExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Mutex, OnceLock},
};

const PROJECT_PATH_HANDOFF: &str = "FARCASTER_CAPTURED_PROJECT_PATH";
const START_MARKER: &[u8] = b"\x1eFARCASTER_ENV_START\x1f\0";
const END_MARKER: &[u8] = b"\x1eFARCASTER_ENV_END\x1f\0";
const CAPTURE_COMMAND: &str = "/bin/sh -c \"command stty -echo -opost; command printf '\\\\036FARCASTER_ENV_START\\\\037\\\\0'; command env -0; command printf '\\\\036FARCASTER_ENV_END\\\\037\\\\0'\" 2>/dev/null; exit\n";

pub(crate) type Environment = Vec<(OsString, OsString)>;

static SHELL_ENVIRONMENTS: OnceLock<Mutex<HashMap<PathBuf, Environment>>> = OnceLock::new();

pub(crate) fn project_shell_environment(project: &Path) -> Result<Option<Environment>, String> {
    #[cfg(test)]
    {
        let _ = project;
        Ok(None)
    }
    #[cfg(not(test))]
    {
        shell_environment_at(project)
            .map(with_project_path_handoff)
            .map(Some)
    }
}

fn with_project_path_handoff(mut environment: Environment) -> Environment {
    let path = environment
        .iter()
        .find(|(name, _)| name == "PATH")
        .map(|(_, value)| value.clone());
    environment.retain(|(name, _)| name != PROJECT_PATH_HANDOFF);
    if let Some(path) = path {
        environment.push((OsString::from(PROJECT_PATH_HANDOFF), path));
    }
    environment
}

pub(crate) fn app_shell_environment() -> Result<Environment, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_owned())?;
    shell_environment_at(&home)
}

fn shell_environment_at(working_directory: &Path) -> Result<Environment, String> {
    let key = working_directory
        .canonicalize()
        .unwrap_or_else(|_| working_directory.to_path_buf());
    let environments = SHELL_ENVIRONMENTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut environments = environments
        .lock()
        .map_err(|_| "shell environment cache is poisoned".to_owned())?;
    if let Some(environment) = environments.get(&key) {
        return Ok(environment.clone());
    }
    let environment = capture_login_shell_environment(&default_login_shell(), working_directory)?;
    environments.insert(key, environment.clone());
    Ok(environment)
}

pub(crate) fn default_login_shell() -> PathBuf {
    #[cfg(target_os = "macos")]
    let shell = account_login_shell().or_else(|| std::env::var_os("SHELL").map(PathBuf::from));
    #[cfg(target_os = "linux")]
    let shell = account_login_shell().or_else(|| std::env::var_os("SHELL").map(PathBuf::from));

    shell.unwrap_or_else(|| {
        if cfg!(target_os = "macos") {
            PathBuf::from("/bin/zsh")
        } else {
            PathBuf::from("/bin/sh")
        }
    })
}

#[cfg(target_os = "macos")]
fn account_login_shell() -> Option<PathBuf> {
    let output = Command::new("/usr/bin/id").arg("-P").output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_account_login_shell(&output.stdout)
}

#[cfg(target_os = "linux")]
fn account_login_shell() -> Option<PathBuf> {
    use uzers::{get_effective_uid, get_user_by_uid, os::unix::UserExt as _};

    let user = get_user_by_uid(get_effective_uid())?;
    let shell = user.shell();
    shell.is_absolute().then(|| shell.to_path_buf())
}

#[cfg(any(target_os = "macos", test))]
fn parse_account_login_shell(output: &[u8]) -> Option<PathBuf> {
    let shell = output
        .trim_ascii_end()
        .rsplit(|byte| *byte == b':')
        .next()?;
    absolute_path(shell)
}

fn absolute_path(value: &[u8]) -> Option<PathBuf> {
    if value.first() != Some(&b'/') {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(value.to_vec())))
}

fn capture_login_shell_environment(shell: &Path, project: &Path) -> Result<Environment, String> {
    let mut command = script_command(shell)?;
    command
        .current_dir(project)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command
        .spawn()
        .map_err(|error| format!("start login shell {} in terminal: {error}", shell.display()))?;
    let mut child = child;
    let _input = {
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| "login shell terminal did not expose input".to_owned())?;
        input
            .write_all(CAPTURE_COMMAND.as_bytes())
            .and_then(|()| input.flush())
            .map_err(|error| format!("request login shell environment: {error}"))?;
        input
    };
    let output = child
        .wait_with_output()
        .map_err(|error| format!("wait for login shell {}: {error}", shell.display()))?;
    if !output.status.success() {
        return Err(format!(
            "login shell {} exited with {}",
            shell.display(),
            output.status
        ));
    }
    parse_environment(&output.stdout)
}

#[cfg(target_os = "macos")]
fn script_command(shell: &Path) -> Result<Command, String> {
    let mut command = Command::new("/usr/bin/script");
    command
        .args(["-q", "/dev/null"])
        .arg(shell)
        .args(["-l", "-i"]);
    Ok(command)
}

#[cfg(target_os = "linux")]
fn script_command(shell: &Path) -> Result<Command, String> {
    let shell = shell
        .to_str()
        .ok_or_else(|| format!("login shell path is not UTF-8: {}", shell.display()))?;
    let quoted_shell = format!("'{}'", shell.replace('\'', "'\\''"));
    let mut command = Command::new("script");
    command
        .args(["-q", "-c"])
        .arg(format!("exec {quoted_shell} -l -i"))
        .arg("/dev/null");
    Ok(command)
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
        assert!(
            parse_environment(&[START_MARKER, b"HOME=/home/user\0", END_MARKER].concat()).is_err()
        );
    }
}
