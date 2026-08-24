//! Login-shell environment import for app and project processes.

#[cfg(target_os = "linux")]
use std::io::Write as _;

use std::{
    collections::HashMap,
    ffi::OsString,
    os::unix::ffi::OsStringExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Mutex, OnceLock},
};

const IMPORT_REQUESTED: &str = "PI_GUI_IMPORT_SHELL_ENV";
const START_MARKER: &[u8] = b"\x1ePI_GPUI_ENV_START\x1f\0";
const END_MARKER: &[u8] = b"\x1ePI_GPUI_ENV_END\x1f\0";
const CAPTURE_COMMAND: &str = "/bin/sh -c \"command stty -echo -opost; command printf '\\\\036PI_GPUI_ENV_START\\\\037\\\\0'; command env -0; command printf '\\\\036PI_GPUI_ENV_END\\\\037\\\\0'\" 2>/dev/null; exit\n";

type Environment = Vec<(OsString, OsString)>;

static SHELL_ENVIRONMENTS: OnceLock<Mutex<HashMap<PathBuf, Environment>>> = OnceLock::new();

pub(crate) fn project_shell_environment(project: &Path) -> Result<Option<Environment>, String> {
    if !environment_import_requested() {
        return Ok(None);
    }
    shell_environment_at(project).map(Some)
}

fn app_shell_environment() -> Result<Option<Environment>, String> {
    if !environment_import_requested() {
        return Ok(None);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_owned())?;
    shell_environment_at(&home).map(Some)
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

fn environment_import_requested() -> bool {
    std::env::var(IMPORT_REQUESTED).as_deref() == Ok("1")
}

pub(crate) fn import_app_shell_environment() -> Result<(), String> {
    let Some(environment) = app_shell_environment()? else {
        return Ok(());
    };
    let inherited_names = std::env::vars_os()
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    // SAFETY: `main` calls this before GPUI or any application worker threads start.
    unsafe {
        for name in inherited_names {
            std::env::remove_var(name);
        }
        for (name, value) in environment {
            std::env::set_var(name, value);
        }
    }
    Ok(())
}

pub(crate) fn terminal_login_shell_command() -> String {
    let shell = std::env::var_os("PI_GUI_SHELL")
        .map(PathBuf::from)
        .unwrap_or_else(default_login_shell);
    login_shell_command(&shell)
}

fn login_shell_command(shell: &Path) -> String {
    format!("'{}' -l", shell.to_string_lossy().replace('\'', "'\\''"))
}

fn default_login_shell() -> PathBuf {
    #[cfg(target_os = "macos")]
    let shell = account_login_shell().or_else(|| std::env::var_os("SHELL").map(PathBuf::from));
    #[cfg(target_os = "linux")]
    let shell = std::env::var_os("SHELL")
        .map(PathBuf::from)
        .or_else(account_login_shell);

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
    let user = std::env::var("USER").ok()?;
    let passwd = std::fs::read("/etc/passwd").ok()?;
    parse_passwd_login_shell(&passwd, &user)
}

fn parse_passwd_login_shell(passwd: &[u8], user: &str) -> Option<PathBuf> {
    passwd.split(|byte| *byte == b'\n').find_map(|record| {
        let mut fields = record.split(|byte| *byte == b':');
        if fields.next() != Some(user.as_bytes()) {
            return None;
        }
        fields.nth(5).and_then(absolute_path)
    })
}

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
    #[cfg(target_os = "linux")]
    let mut child = child;
    #[cfg(target_os = "linux")]
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
        .args(["-l", "-i", "-c", CAPTURE_COMMAND]);
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
    fn login_shell_command_quotes_the_executable_path() {
        assert_eq!(
            login_shell_command(Path::new("/tmp/my shell's bin")),
            "'/tmp/my shell'\\''s bin' -l"
        );
    }

    #[test]
    fn account_record_yields_its_absolute_login_shell() {
        assert_eq!(
            parse_account_login_shell(b"user:*:501:20::0:0:User:/Users/user:/opt/bin/fish\n"),
            Some(PathBuf::from("/opt/bin/fish"))
        );
        assert_eq!(
            parse_passwd_login_shell(
                b"other:x:1000:1000::/home/other:/bin/bash\nuser:x:1001:100::/home/user:/bin/fish\n",
                "user",
            ),
            Some(PathBuf::from("/bin/fish"))
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn capture_runs_a_real_interactive_login_shell() -> TestResult {
        let temp = tempdir()?;
        let shell = temp.path().join("shell");
        #[cfg(target_os = "macos")]
        let run_capture = r#"
test "$3" = "-c"
test "$#" = "4"
exec /bin/sh -c "$4"
"#;
        #[cfg(target_os = "linux")]
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

    #[test]
    fn parser_rejects_incomplete_or_pathless_output() {
        assert!(parse_environment(START_MARKER).is_err());
        assert!(
            parse_environment(&[START_MARKER, b"HOME=/home/user\0", END_MARKER].concat()).is_err()
        );
    }
}
