use std::io::{Read as _, Write as _};

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

#[cfg(any(target_os = "macos", test))]
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
        // Avoid an undrained script pipe; shell diagnostics travel through the PTY.
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| format!("start login shell {} in terminal: {error}", shell.display()))?;
    let output = capture_terminal_output(&mut child);
    if output.is_err() {
        let _ = child.kill();
    }
    let status = child.wait();
    let output = output?;
    let status =
        status.map_err(|error| format!("wait for login shell {}: {error}", shell.display()))?;
    if !status.success() {
        return Err(format!(
            "login shell {} exited with {status}",
            shell.display(),
        ));
    }
    parse_environment(&output)
}

fn capture_terminal_output(child: &mut std::process::Child) -> Result<Vec<u8>, String> {
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| "login shell terminal did not expose input".to_owned())?;
    let mut output = child
        .stdout
        .take()
        .ok_or_else(|| "login shell terminal did not expose output".to_owned())?;
    input
        .write_all(CAPTURE_COMMAND.as_bytes())
        .and_then(|()| input.flush())
        .map_err(|error| format!("request login shell environment: {error}"))?;

    let mut terminal = CaptureTerminal::default();
    let mut captured = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = match output.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("read login shell environment: {error}")),
        };
        terminal
            .respond(&buffer[..count], &mut input)
            .map_err(|error| format!("answer login shell terminal query: {error}"))?;
        captured.extend_from_slice(&buffer[..count]);
    }
    Ok(captured)
}

/// Complete fish's terminal-query barrier without advertising optional features
/// or changing shell initialization. This is not a full terminal emulator.
#[derive(Default)]
struct CaptureTerminal {
    tail: Vec<u8>,
    capturing_environment: bool,
}

impl CaptureTerminal {
    fn respond(&mut self, output: &[u8], input: &mut impl std::io::Write) -> std::io::Result<()> {
        for &byte in output {
            if self.capturing_environment {
                break;
            }
            self.tail.push(byte);
            if self.tail.ends_with(START_MARKER) {
                // Exported values are data, even if they contain escape sequences.
                self.capturing_environment = true;
                break;
            }
            if self.tail.ends_with(b"\x1b[0c") || self.tail.ends_with(b"\x1b[c") {
                input.write_all(b"\x1b[?0c")?;
                input.flush()?;
            }
            if self.tail.len() >= START_MARKER.len() {
                self.tail.remove(0);
            }
        }
        Ok(())
    }
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
#[path = "shell_environment_tests.rs"]
mod tests;
