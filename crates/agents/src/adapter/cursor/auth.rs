use std::{
    ffi::OsString,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::AgentLaunchConfig;
use serde::Deserialize;

const LOGIN_HELPER: &str = include_str!("auth.mjs");

pub(super) fn environment(command: &Command, name: &str) -> Option<OsString> {
    match command.get_envs().find(|(key, _)| *key == name) {
        Some((_, value)) => value.map(OsString::from),
        None => std::env::var_os(name),
    }
}

fn credential_path(config: &AgentLaunchConfig, command: &Command) -> Result<PathBuf, String> {
    if config.profile_id.is_some() {
        return config
            .locator_root()
            .map(|root| root.join("cursor-sdk-auth/auth.json"))
            .ok_or_else(|| "Profile credential directory is unavailable".into());
    }
    environment(command, "HOME")
        .map(|home| PathBuf::from(home).join(".cursor/sdk/auth.json"))
        .ok_or_else(|| "HOME is required for Cursor sign-in".into())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Credentials {
    version: u32,
    backend_url: String,
    api_key: String,
    api_key_expires_at_ms: Option<f64>,
    created_at_ms: f64,
}

fn saved_key(path: &Path, backend: &str, now_ms: f64) -> Result<Option<String>, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Read Cursor sign-in: {error}")),
    };
    let mut bytes = Vec::new();
    file.take(65537)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Read Cursor sign-in: {error}"))?;
    let Ok(credentials) = serde_json::from_slice::<Credentials>(&bytes) else {
        return Ok(None);
    };
    Ok((credentials.version == 1
        && credentials.backend_url.trim_end_matches('/') == backend.trim_end_matches('/')
        && credentials.created_at_ms.is_finite()
        && !credentials.api_key.trim().is_empty()
        && credentials
            .api_key_expires_at_ms
            .is_none_or(|expires| expires > now_ms))
    .then_some(credentials.api_key))
}

pub(super) fn api_key(
    config: &AgentLaunchConfig,
    command: &Command,
) -> Result<Option<String>, String> {
    if let Some(key) = environment(command, "CURSOR_API_KEY")
        .map(|key| key.to_string_lossy().into_owned())
        .filter(|key| !key.trim().is_empty())
    {
        return Ok(Some(key));
    }
    let backend = environment(command, "CURSOR_BACKEND_URL")
        .map(|url| url.to_string_lossy().into_owned())
        .unwrap_or_else(|| "https://api2.cursor.sh".into());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    saved_key(
        &credential_path(config, command)?,
        &backend,
        now.as_secs_f64() * 1000.0,
    )
}

pub(super) fn sign_in_required(config: &AgentLaunchConfig, command: &Command) -> bool {
    !api_key(config, command).is_ok_and(|key| key.is_some())
}

pub(super) fn sign_in(
    config: &AgentLaunchConfig,
    project: &Path,
    cancelled: &AtomicBool,
    on_url: &dyn Fn(String),
) -> Result<(), String> {
    let environment = config.command(project)?;
    let store = credential_path(config, &environment)?;
    let mut command = sdk_command(&environment, project, LOGIN_HELPER)?;
    command.arg(store);
    run(command, cancelled, on_url, Duration::from_secs(310))
}

pub(super) fn sdk_command(
    environment: &Command,
    project: &Path,
    script: &str,
) -> Result<Command, String> {
    let sdk = super::installation_dir()
        .ok_or("HOME is required for the Cursor SDK")?
        .join("node_modules/@cursor/sdk/dist/esm/index.js");
    if !sdk.is_file() {
        return Err("Install the Cursor SDK with scripts/install-cursor-sdk.sh".into());
    }
    let node = super::super::process_command::resolve_agent_program(
        Path::new("node"),
        project,
        self::environment(&environment, "PATH").as_deref(),
    )?;
    let mut command = Command::new(node);
    command.current_dir(project);
    for (key, value) in environment.get_envs() {
        match value {
            Some(value) => {
                command.env(key, value);
            }
            None => {
                command.env_remove(key);
            }
        }
    }
    command
        .args(["--input-type=module", "-e", &super::timing::script(script)])
        .arg(sdk);
    Ok(command)
}

struct LoginProcess(Child);
impl Drop for LoginProcess {
    fn drop(&mut self) {
        // Closing stdin also aborts the SDK's pending browser login.
        self.0.stdin.take();
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(self.0.try_wait(), Ok(None)) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn run(
    mut command: Command,
    cancelled: &AtomicBool,
    on_url: &dyn Fn(String),
    timeout: Duration,
) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) {
        return Err("Sign-in cancelled".into());
    }
    let mut child = LoginProcess(
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("Start Cursor sign-in: {error}"))?,
    );
    let stdout = child.0.stdout.take().expect("piped login stdout");
    let (sender, receiver) = mpsc::sync_channel(8);
    thread::Builder::new()
        .name("cursor-sign-in-output".into())
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.by_ref().take(65537).read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line.len() > 65536 => break,
                    Ok(_) => {
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                }
            }
        })
        .map_err(|error| format!("Read Cursor sign-in: {error}"))?;
    let deadline = Instant::now() + timeout;
    loop {
        if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
            if let Some(stdin) = child.0.stdin.as_mut() {
                let _ = stdin.write_all(b"cancel\n");
            }
            return Err(if cancelled.load(Ordering::Acquire) {
                "Sign-in cancelled"
            } else {
                "Cursor sign-in timed out"
            }
            .into());
        }
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => {
                let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                match event["type"].as_str() {
                    Some("timing") => super::timing::log_event(&event),
                    Some("url") => {
                        let url = event["url"]
                            .as_str()
                            .ok_or("Cursor omitted the sign-in URL")?;
                        let parsed = url::Url::parse(url)
                            .map_err(|_| "Cursor returned an invalid sign-in URL")?;
                        if !matches!(parsed.scheme(), "https" | "http") {
                            return Err("Cursor returned an invalid sign-in URL".into());
                        }
                        on_url(url.to_owned());
                    }
                    Some("complete") => return Ok(()),
                    Some("error") => {
                        return Err(event["message"]
                            .as_str()
                            .unwrap_or("Cursor sign-in failed")
                            .to_owned());
                    }
                    _ => {}
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Cursor sign-in ended before completion".into());
            }
        }
    }
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
