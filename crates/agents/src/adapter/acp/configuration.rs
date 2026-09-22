use crate::Backend;
use std::{
    collections::HashMap,
    ffi::OsString,
    process::Command,
    sync::{Mutex, OnceLock},
};

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::super::main_session::MainSessionMetadata;
use super::{
    AcpProfile,
    connection::AcpConnection,
    translate::{ConfigIds, metadata_from_session},
};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct AcpRuntimeKey {
    program: OsString,
    environment_digest: [u8; 32],
}

impl AcpRuntimeKey {
    pub(super) fn from_command(command: &Command) -> Self {
        let mut environment = command
            .get_envs()
            .filter(|(name, _)| *name != "PWD" && *name != "OLDPWD")
            .map(|(name, value)| (name.to_owned(), value.map(OsString::from)))
            .collect::<Vec<_>>();
        environment.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        let mut digest = Sha256::new();
        for (name, value) in environment {
            let name = name.as_encoded_bytes();
            digest.update(name.len().to_le_bytes());
            digest.update(name);
            if let Some(value) = value {
                let value = value.as_encoded_bytes();
                digest.update([1]);
                digest.update(value.len().to_le_bytes());
                digest.update(value);
            } else {
                digest.update([0]);
            }
        }
        Self {
            program: command.get_program().to_owned(),
            environment_digest: digest.finalize().into(),
        }
    }
}

pub(super) fn model_catalog(
    connection: &AcpConnection,
    profile: &AcpProfile,
    key: &AcpRuntimeKey,
) -> Result<Vec<Value>, String> {
    if profile.backend != Backend::Cursor {
        return Ok(Vec::new());
    }
    static CATALOGS: OnceLock<Mutex<HashMap<AcpRuntimeKey, Vec<Value>>>> = OnceLock::new();
    let catalogs = CATALOGS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(catalog) = catalogs
        .lock()
        .map_err(|error| format!("Cursor model catalog cache: {error}"))?
        .get(key)
        .cloned()
    {
        return Ok(catalog);
    }
    let catalog = connection.request_model_catalog(profile)?;
    catalogs
        .lock()
        .map_err(|error| format!("Cursor model catalog cache: {error}"))?
        .insert(key.clone(), catalog.clone());
    Ok(catalog)
}

pub(super) const CURSOR_SERVICE_TIERS: [(&str, &str); 2] =
    [("standard", "false"), ("priority", "true")];

fn cursor_service_tier(value: &Value) -> Option<String> {
    let value = value.as_str()?;
    CURSOR_SERVICE_TIERS
        .iter()
        .find(|(_, native)| *native == value)
        .map(|(tier, _)| (*tier).to_owned())
}

#[derive(Clone)]
pub(super) struct ModelSelection {
    pub model: String,
    pub parameters: Vec<(String, String)>,
}

/// Keep effort and service tier separate; expand remaining model parameters
/// into model choices without exposing native configuration commands.
pub(super) fn metadata(
    profile: &AcpProfile,
    response: &Value,
    catalog: Vec<Value>,
) -> (MainSessionMetadata, ConfigIds) {
    let (mut metadata, mut ids) = metadata_from_session(profile, response);
    let current_options = response
        .get("configOptions")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if profile.backend == Backend::Cursor
        && let Some(fast) = current_options
            .iter()
            .find(|option| option.get("id").and_then(Value::as_str) == Some("fast"))
    {
        ids.service_tier = Some("fast".into());
        metadata.service_tiers = fast
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|option| option.get("value").and_then(cursor_service_tier))
            .collect();
        metadata.service_tier = fast.get("currentValue").and_then(cursor_service_tier);
        ids.selected_service_tier = metadata.service_tier.clone();
        ids.service_tiers = metadata.service_tiers.clone();
    }
    let current_model = current_options
        .iter()
        .find(|option| option.get("id").and_then(Value::as_str) == ids.model.as_deref())
        .and_then(|option| option.get("currentValue"))
        .and_then(Value::as_str);
    if catalog.is_empty() {
        ids.selected_model = current_model.map(str::to_owned);
        return (metadata, ids);
    }
    let current: HashMap<_, _> = current_options
        .iter()
        .filter_map(|option| {
            Some((
                option.get("id")?.as_str()?,
                option.get("currentValue")?.as_str()?,
            ))
        })
        .collect();
    let mut models = Vec::new();
    let mut selected = None;
    for model in &catalog {
        let Some(base) = model.get("value").and_then(Value::as_str) else {
            continue;
        };
        let name = model.get("name").and_then(Value::as_str).unwrap_or(base);
        let options = model
            .get("configOptions")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let (parameters_metadata, _) = super::translate::metadata_from_options(profile, options);
        let efforts = parameters_metadata.efforts;
        let mut combinations: Vec<Vec<(String, String)>> = vec![Vec::new()];
        for option in options {
            if profile.backend == Backend::Cursor
                && option.get("id").and_then(Value::as_str) == Some("fast")
            {
                continue;
            }
            if option.get("category").and_then(Value::as_str) != Some("model_config") {
                continue;
            }
            let Some(id) = option.get("id").and_then(Value::as_str) else {
                continue;
            };
            let Some(values) = option.get("options").and_then(Value::as_array) else {
                continue;
            };
            combinations = combinations
                .into_iter()
                .flat_map(|parameters| {
                    values.iter().filter_map(move |value| {
                        let value = value.get("value")?.as_str()?;
                        let mut parameters = parameters.clone();
                        parameters.push((id.to_owned(), value.to_owned()));
                        Some(parameters)
                    })
                })
                .collect();
        }
        for parameters in combinations {
            let suffix = parameters
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(",");
            let id = if suffix.is_empty() {
                base.to_owned()
            } else {
                format!("{base}[{suffix}]")
            };
            if Some(base) == current_model
                && parameters
                    .iter()
                    .all(|(key, value)| current.get(key.as_str()) == Some(&value.as_str()))
            {
                selected = Some(models.len());
            }
            let context_window = parameters
                .iter()
                .find(|(key, _)| key == "context")
                .and_then(|(_, value)| {
                    let (number, multiplier) = if let Some(n) = value.strip_suffix('k') {
                        (n, 1_000)
                    } else if let Some(n) = value.strip_suffix('m') {
                        (n, 1_000_000)
                    } else {
                        (value.as_str(), 1)
                    };
                    number.parse::<u64>().ok()?.checked_mul(multiplier)
                })
                .unwrap_or(0);
            models.push(json!({"id":id,"name":if suffix.is_empty() {name.to_owned()} else {format!("{name} · {suffix}")},
                "provider":profile.backend,"contextWindow":context_window,"reasoning":!efforts.is_empty(),"efforts":efforts}));
            ids.selections.insert(
                id,
                ModelSelection {
                    model: base.into(),
                    parameters,
                },
            );
        }
    }
    if let Some(index) = selected {
        models.swap(0, index);
        ids.selected_model = models[0]
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned);
    }
    metadata.models = models;
    ids.catalog = catalog;
    (metadata, ids)
}

#[cfg(test)]
#[path = "configuration_tests.rs"]
mod tests;
