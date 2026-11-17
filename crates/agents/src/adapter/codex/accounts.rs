use super::super::process_command::environment;
use super::{
    connection::read_message,
    contract::{CodexInbound, CodexRequestId},
    wire::encode_request,
};
use crate::{AgentLaunchConfig, Backend, accounts::*};
use serde_json::{Value, json};
use std::{
    io::{BufReader, Write as _},
    path::Path,
    process::{Child, ChildStdin, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub(super) fn capabilities() -> AccountCapabilities {
    AccountCapabilities {
        discover: true,
        identity: true,
        limits: true,
        login: true,
        logout: true,
        external_changes: false,
    }
}

struct AccountProcess {
    child: Child,
    input: ChildStdin,
    frames: mpsc::Receiver<Result<CodexInbound, String>>,
    next_id: i64,
}
impl Drop for AccountProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl AccountProcess {
    fn start(config: &AgentLaunchConfig, project: &Path) -> Result<Self, String> {
        let command = super::super::configuration_launch(config, Backend::Codex)?;
        let mut command = command.command(project)?;
        command.arg("app-server");
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Start Codex account connection: {e}"))?;
        let input = child.stdin.take().ok_or("Codex account input missing")?;
        let output = child.stdout.take().ok_or("Codex account output missing")?;
        let (tx, frames) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let frame = read_message(&mut reader);
                let failed = frame.is_err();
                if tx.send(frame).is_err() || failed {
                    break;
                }
            }
        });
        let mut process = Self {
            child,
            input,
            frames,
            next_id: 0,
        };
        process.request("initialize", json!({"clientInfo":{"name":"farcaster", "version":env!("CARGO_PKG_VERSION")}, "capabilities":{"experimentalApi":true}}))?;
        process
            .input
            .write_all(b"{\"method\":\"initialized\"}\n")
            .map_err(|e| e.to_string())?;
        process.input.flush().map_err(|e| e.to_string())?;
        Ok(process)
    }
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = CodexRequestId::Number(self.next_id);
        self.input
            .write_all(&encode_request(&id, method, params)?)
            .and_then(|()| self.input.flush())
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match self
                .frames
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| "Codex account request timed out")??
            {
                CodexInbound::Response {
                    id: response,
                    result,
                } if response == id => return Ok(result),
                CodexInbound::Error {
                    id: response,
                    error,
                } if response == id => {
                    return Err(format!("Codex account request failed ({})", error.code));
                }
                _ => {}
            }
        }
    }
}

pub(super) fn source(config: &AgentLaunchConfig, project: &Path) -> Result<AccountSource, String> {
    let launch = super::super::configuration_launch(config, Backend::Codex)?;
    let command = launch.command(project)?;
    let home = environment(&command, "CODEX_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            environment(&command, "HOME").map(|p| std::path::PathBuf::from(p).join(".codex"))
        });
    let credential_source = format!("native:{}", home.unwrap_or_default().display());
    Ok(super::super::accounts::source(
        config,
        Backend::Codex,
        project,
        "openai",
        credential_source,
    ))
}

pub(super) fn read(config: &AgentLaunchConfig, project: &Path) -> Result<AccountDiscovery, String> {
    let source = source(config, project)?;
    let mut process = AccountProcess::start(config, project)?;
    let account = process.request("account/read", json!({"refreshToken":false}))?;
    let now = account_now();
    let mut observation = observation(source, &account["account"], now);
    let limits = if account["account"]["type"].as_str() == Some("chatgpt") {
        process
            .request("account/rateLimits/read", Value::Null)
            .ok()
            .filter(|v| v["rateLimits"].is_object() || v["rateLimitsByLimitId"].is_object())
            .map(|v| limits(&v, now))
    } else {
        Some(Vec::new())
    };
    observation.complete_limits = limits.is_some();
    observation.limits_failed = limits.is_none();
    observation.limits = limits.unwrap_or_default();
    Ok(AccountDiscovery {
        source: observation.source.clone(),
        observations: vec![observation],
    })
}

pub(super) fn observation(source: AccountSource, account: &Value, now: i64) -> AccountObservation {
    let auth = if account.is_null() {
        AccountAuthState::SignedOut
    } else if account["type"] == "chatgpt" {
        AccountAuthState::SignedIn
    } else {
        AccountAuthState::Configured
    };
    AccountObservation {
        source,
        identity: identity(account),
        auth,
        complete_limits: false,
        limits_failed: false,
        limits: Vec::new(),
        observed_at: now,
        capabilities: capabilities(),
    }
}

pub(super) fn identity(value: &Value) -> Option<AccountIdentity> {
    if value.is_null() {
        return None;
    }
    Some(AccountIdentity {
        authority: "openai".into(),
        principal_id: text(value, "userId"),
        workspace_id: text(value, "accountId").or_else(|| text(value, "chatgptAccountId")),
        email: text(value, "email"),
        label: value["type"]
            .as_str()
            .filter(|t| *t != "chatgpt")
            .map(|_| "OpenAI API".into()),
        workspace: text(value, "workspaceName"),
        plan: text(value, "planType"),
    })
}
fn text(value: &Value, key: &str) -> Option<String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub(super) fn limits(value: &Value, now: i64) -> Vec<AccountLimit> {
    let buckets: Vec<_> = value["rateLimitsByLimitId"]
        .as_object()
        .map(|m| m.iter().map(|(id, v)| (id.as_str(), v)).collect())
        .unwrap_or_else(|| vec![("codex", value.get("rateLimits").unwrap_or(value))]);
    let mut result = Vec::new();
    for (id, bucket) in buckets {
        for slot in ["primary", "secondary"] {
            let window = &bucket[slot];
            let Some(used) = window["usedPercent"]
                .as_f64()
                .filter(|p| p.is_finite() && (0.0..=100.0).contains(p))
            else {
                continue;
            };
            result.push(AccountLimit {
                key: format!("{id}:{slot}"),
                label: text(bucket, "limitName").unwrap_or_else(|| id.into()),
                pool_id: None,
                duration_seconds: window["windowDurationMins"]
                    .as_u64()
                    .and_then(|m| m.checked_mul(60)),
                remaining_percent: Some(100.0 - used),
                value: None,
                resets_at: window["resetsAt"].as_i64(),
                observed_at: now,
            });
        }
        let credits = &bucket["credits"];
        let value = if credits["unlimited"] == true {
            Some(AccountQuotaValue::Unlimited)
        } else {
            credits["balance"]
                .as_f64()
                .or_else(|| {
                    credits["balance"]
                        .as_str()
                        .and_then(|s| s.parse::<f64>().ok())
                })
                .filter(|n| n.is_finite())
                .map(|remaining| AccountQuotaValue::Balance {
                    remaining,
                    unit: "credits".into(),
                })
        };
        if let Some(value) = value {
            result.push(AccountLimit {
                key: format!("{id}:credits"),
                label: "Credits".into(),
                pool_id: None,
                duration_seconds: None,
                remaining_percent: None,
                value: Some(value),
                resets_at: None,
                observed_at: now,
            });
        }
    }
    result
}

pub(super) fn login(
    config: &AgentLaunchConfig,
    project: &Path,
    cancelled: &AtomicBool,
    on_url: &dyn Fn(String),
) -> Result<(), String> {
    let mut process = AccountProcess::start(config, project)?;
    let result = process.request("account/login/start", json!({"type":"chatgpt"}))?;
    if let Some(url) = result["authUrl"].as_str() {
        on_url(url.into());
    }
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline {
        if cancelled.load(Ordering::Acquire) {
            let _ = process.request("account/login/cancel", json!({"loginId":result["loginId"]}));
            return Err("Sign-in cancelled".into());
        }
        match process.frames.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(CodexInbound::Notification { method, params }))
                if method == "account/login/completed"
                    && params["loginId"] == result["loginId"] =>
            {
                return if params["success"] == true {
                    Ok(())
                } else {
                    Err("Codex sign-in did not complete".into())
                };
            }
            Ok(Err(error)) => return Err(error),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Codex account connection closed".into());
            }
            _ => {}
        }
    }
    Err("Codex sign-in timed out".into())
}

pub(super) fn logout(
    config: &AgentLaunchConfig,
    expected_source: &AccountSource,
    expected_identity: &AccountIdentity,
    project: &Path,
) -> Result<(), String> {
    if &source(config, project)? != expected_source {
        return Err("Connection changed. Refresh accounts before signing out.".into());
    }
    let mut process = AccountProcess::start(config, project)?;
    let current = process.request("account/read", json!({"refreshToken":false}))?;
    if identity(&current["account"]).as_ref() != Some(expected_identity) {
        return Err("Login changed. Refresh accounts before signing out.".into());
    }
    process.request("account/logout", Value::Null).map(|_| ())
}

#[cfg(test)]
#[path = "accounts_tests.rs"]
mod tests;
