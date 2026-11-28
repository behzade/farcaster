use super::super::{acp, process_command};
use crate::accounts::AccountCapabilities;
use serde_json::json;
use std::{
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub(super) const METHODS: &[(&str, &str)] = &[
    ("oauth-personal", "Google account"),
    ("oauth-business", "Gemini Enterprise"),
    ("terminal-login", "Terminal login"),
    ("gemini-api-key", "Gemini API key"),
    ("agent-platform", "Enterprise Agent Platform"),
];

pub(super) fn capabilities(command: &Command) -> AccountCapabilities {
    let environment_login = ["GEMINI_API_KEY", "GOOGLE_APPLICATION_CREDENTIALS"]
        .iter()
        .any(|key| process_command::environment(command, key).is_some_and(|v| !v.is_empty()));
    AccountCapabilities {
        identity: true,
        login: super::integrated_cli(command),
        logout: super::integrated_cli(command) && !environment_login,
        ..Default::default()
    }
}

pub(super) fn request(
    mut command: Command,
    method: Option<&str>,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    if !super::integrated_cli(&command) {
        return Err("Antigravity account controls require the agy CLI".into());
    }
    super::configure(&mut command, crate::HarnessAccessMode::Sandboxed)?;
    let (mut child, mut connection) = acp::catalog::spawn_catalog_child(&super::PROFILE, command)?;
    let result = (|| {
        let init = connection.initialize(&super::PROFILE)?;
        let id = if let Some(method) = method {
            if !init["authMethods"]
                .as_array()
                .is_some_and(|methods| methods.iter().any(|m| m["id"] == method))
            {
                return Err(
                    "The installed Antigravity CLI does not offer this sign-in method".into(),
                );
            }
            connection.send_request("authenticate", json!({"methodId":method}))?
        } else {
            if !init
                .pointer("/agentCapabilities/auth/logout")
                .is_some_and(|v| v.is_object())
            {
                return Err("This Antigravity version does not offer logout".into());
            }
            connection.send_request("logout", json!({}))?
        };
        let start = std::time::Instant::now();
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err("Sign-in cancelled".into());
            }
            if start.elapsed() > Duration::from_secs(600) {
                return Err("Antigravity account request timed out".into());
            }
            if let Some(incoming) = connection.poll() {
                match incoming? {
                    acp::events::AcpInbound::Response { id: reply, .. } if reply == id => {
                        return Ok(());
                    }
                    acp::events::AcpInbound::Error {
                        id: reply,
                        rejection,
                    } if reply == id => return Err(rejection.to_string()),
                    _ => {}
                }
            } else if child.try_wait().map_err(|err| err.to_string())?.is_some() {
                return Err("Antigravity exited during authentication".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    })();
    drop(connection);
    let _ = child.kill();
    let _ = child.wait();
    result
}
