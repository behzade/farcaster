use std::collections::HashMap;

use serde_json::{Value, json};

use super::super::main_session::MainSessionMetadata;
use super::{
    AcpProfile,
    translate::{ConfigIds, metadata_from_session},
};

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
    if profile.backend == "cursor-cli"
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
    if catalog.is_empty() {
        return (metadata, ids);
    }
    let current_model = current_options
        .iter()
        .find(|option| option.get("id").and_then(Value::as_str) == ids.model.as_deref())
        .and_then(|option| option.get("currentValue"))
        .and_then(Value::as_str);
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
            if profile.backend == "cursor-cli"
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
