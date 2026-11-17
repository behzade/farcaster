use super::super::process_command::environment;
use crate::{AgentLaunchConfig, Backend, accounts::*};
use serde_json::{Value, json};
use std::path::Path;

pub(super) fn capabilities() -> AccountCapabilities {
    AccountCapabilities {
        discover: true,
        identity: true,
        limits: true,
        ..Default::default()
    }
}

pub(super) fn source(config: &AgentLaunchConfig, project: &Path) -> Result<AccountSource, String> {
    let launch = super::super::configuration_launch(config, Backend::Claude)?;
    let command = launch.command(project)?;
    let env = |name| environment(&command, name);
    let provider = [
        ("CLAUDE_CODE_USE_BEDROCK", "bedrock"),
        ("CLAUDE_CODE_USE_VERTEX", "vertex"),
        ("CLAUDE_CODE_USE_FOUNDRY", "foundry"),
    ]
    .into_iter()
    .find(|(key, _)| env(key).is_some_and(|v| v == "1"))
    .map(|(_, provider)| provider)
    .unwrap_or("anthropic");
    let credential_source = if env("ANTHROPIC_API_KEY").is_some() {
        "environment:ANTHROPIC_API_KEY".into()
    } else if env("CLAUDE_CODE_OAUTH_TOKEN").is_some() {
        "environment:CLAUDE_CODE_OAUTH_TOKEN".into()
    } else {
        format!(
            "native:{}",
            env("CLAUDE_CONFIG_DIR")
                .map(std::path::PathBuf::from)
                .or_else(|| env("HOME").map(|v| std::path::PathBuf::from(v).join(".claude")))
                .unwrap_or_default()
                .display()
        )
    };
    Ok(super::super::accounts::source(
        config,
        Backend::Claude,
        project,
        provider,
        credential_source,
    ))
}

pub(super) fn read(config: &AgentLaunchConfig, project: &Path) -> Result<AccountDiscovery, String> {
    let launch = super::super::configuration_launch(config, Backend::Claude)?;
    let source = source(config, project)?;
    let mut process = super::process::Process::spawn(
        &launch,
        project,
        &uuid::Uuid::new_v4().to_string(),
        false,
        None,
        None,
        false,
        None,
    )?;
    let response = process.wait(json!({"subtype":"initialize", "supportedDialogKinds":[]}))?;
    let mut observation = observation(source, &response["account"], account_now());
    // Pinned SDK 0.3.257 exposes this explicitly experimental control. Older CLIs may reject it.
    if let Ok(usage) = process.wait(json!({"subtype":"get_usage"}))
        && usage["rate_limits_available"].is_boolean()
    {
        observation.limits = limits(&usage["rate_limits"], observation.observed_at);
        observation.complete_limits = true;
    } else {
        observation.limits_failed = true;
    }
    Ok(AccountDiscovery {
        source: observation.source.clone(),
        observations: vec![observation],
    })
}

pub(super) fn observation(
    mut source: AccountSource,
    value: &Value,
    now: i64,
) -> AccountObservation {
    if let Some(provider) = value["apiProvider"].as_str() {
        source.provider = if provider == "firstParty" {
            "anthropic"
        } else {
            provider
        }
        .into();
    }
    let identity = identity(value, &source.provider);
    let auth = if identity.as_ref().is_some_and(|id| id.email.is_some()) {
        AccountAuthState::SignedIn
    } else if identity.is_some() {
        AccountAuthState::Configured
    } else {
        AccountAuthState::Unknown
    };
    AccountObservation {
        source,
        identity,
        auth,
        limits: Vec::new(),
        complete_limits: false,
        limits_failed: false,
        observed_at: now,
        capabilities: capabilities(),
    }
}

pub(super) fn identity(value: &Value, provider: &str) -> Option<AccountIdentity> {
    let text = |key| {
        value[key]
            .as_str()
            .filter(|s| !s.is_empty() && *s != "none")
            .map(str::to_owned)
    };
    let email = text("email");
    if email.is_none() && text("apiKeySource").is_none() && text("tokenSource").is_none() {
        return None;
    }
    Some(AccountIdentity {
        authority: provider.into(),
        principal_id: None,
        workspace_id: None,
        email,
        label: Some(provider.into()),
        workspace: text("organization"),
        plan: text("subscriptionType"),
    })
}

pub(super) fn limits(value: &Value, now: i64) -> Vec<AccountLimit> {
    let Some(windows) = value.as_object() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut add_window = |key: String, label: String, window: &Value, duration: Option<u64>| {
        let Some(used) = window["utilization"]
            .as_f64()
            .filter(|p| p.is_finite() && (0.0..=100.0).contains(p))
        else {
            return;
        };
        result.push(AccountLimit {
            key,
            label,
            pool_id: None,
            duration_seconds: duration,
            remaining_percent: Some(100.0 - used),
            value: None,
            resets_at: window["resets_at"]
                .as_str()
                .and_then(|s| {
                    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
                        .ok()
                })
                .map(|t| t.unix_timestamp()),
            observed_at: now,
        });
    };
    for (key, window) in windows {
        if key == "model_scoped" {
            if let Some(models) = window.as_array() {
                for model in models {
                    if let Some(name) = model["display_name"].as_str() {
                        add_window(format!("model:{name}"), name.into(), model, Some(604800));
                    }
                }
            }
        } else if key != "extra_usage" {
            let duration = if key.starts_with("seven_day") {
                Some(604800)
            } else if key == "five_hour" {
                Some(18000)
            } else {
                None
            };
            let label = match key.as_str() {
                "seven_day" => "Weekly".into(),
                "five_hour" => "5 hours".into(),
                _ => key.replace('_', " "),
            };
            add_window(key.clone(), label, window, duration);
        }
    }
    let extra = &value["extra_usage"];
    if extra["is_enabled"] == true
        && let Some(used) = extra["used_credits"].as_f64().filter(|v| v.is_finite())
    {
        result.push(AccountLimit {
            key: "extra_usage".into(),
            label: "Extra usage".into(),
            pool_id: None,
            duration_seconds: None,
            remaining_percent: extra["utilization"]
                .as_f64()
                .filter(|p| p.is_finite() && (0.0..=100.0).contains(p))
                .map(|p| 100.0 - p),
            value: Some(AccountQuotaValue::Usage {
                used,
                limit: extra["monthly_limit"].as_f64().filter(|v| v.is_finite()),
                unit: "credits".into(),
            }),
            resets_at: None,
            observed_at: now,
        });
    }
    result
}

#[cfg(test)]
#[path = "accounts_tests.rs"]
mod tests;
