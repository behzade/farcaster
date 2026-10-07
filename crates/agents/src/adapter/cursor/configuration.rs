use crate::adapter::main_session::MainSessionMetadata;
use serde_json::{Value, json};

#[derive(Clone)]
pub(super) struct Model {
    pub(super) id: String,
    pub(super) wire: Value,
    pub(super) display: Value,
    pub(super) effort_parameter: Option<String>,
    pub(super) efforts: Vec<String>,
    pub(super) tiers: Vec<String>,
}

pub(super) fn models(catalog: &[Value]) -> Vec<Model> {
    let mut result = Vec::new();
    for model in catalog {
        let Some(id) = model["id"].as_str() else {
            continue;
        };
        let name = model["displayName"].as_str().unwrap_or(id);
        let mut defaults = Vec::new();
        let mut effort_parameter = None;
        let mut efforts = Vec::new();
        let mut tiers = Vec::new();
        let mut variants = vec![Vec::<Value>::new()];
        for parameter in model["parameters"].as_array().into_iter().flatten() {
            let Some(key) = parameter["id"].as_str() else {
                continue;
            };
            let values: Vec<_> = parameter["values"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v["value"].as_str())
                .collect();
            if key.contains("effort") || key.contains("reasoning") {
                effort_parameter = Some(key.to_owned());
                efforts = values.iter().map(|v| (*v).to_owned()).collect();
                if let Some(value) = values.first() {
                    defaults.push(json!({"id":key,"value":value}));
                }
            } else if key == "fast" {
                tiers = values
                    .iter()
                    .filter_map(|v| match *v {
                        "true" => Some("priority".into()),
                        "false" => Some("standard".into()),
                        _ => None,
                    })
                    .collect();
                if let Some(value) = values.first() {
                    defaults.push(json!({"id":key,"value":value}));
                }
            } else if !values.is_empty() {
                variants = variants
                    .into_iter()
                    .flat_map(|params| {
                        values.iter().map(move |value| {
                            let mut params = params.clone();
                            params.push(json!({"id":key,"value":value}));
                            params
                        })
                    })
                    .collect();
            }
        }
        for parameters in variants {
            let suffix = parameters
                .iter()
                .map(|p| {
                    format!(
                        "{}={}",
                        p["id"].as_str().unwrap_or_default(),
                        p["value"].as_str().unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            let selection_id = if suffix.is_empty() {
                id.to_owned()
            } else {
                format!("{id}[{suffix}]")
            };
            let mut params = defaults.clone();
            params.extend(parameters);
            let wire = json!({"id":id,"params":params});
            let display = json!({
                "id":selection_id,
                "name":if suffix.is_empty() {name.to_owned()} else {format!("{name} · {suffix}")},
                "provider":crate::Backend::Cursor,
                "contextWindow":0,
                "reasoning":!efforts.is_empty(),
                "efforts":efforts,
                "serviceTiers":tiers,
                "adapterData":{"version":1,"model":wire,"effortParameter":effort_parameter}
            });
            result.push(Model {
                id: selection_id,
                wire,
                display,
                effort_parameter: effort_parameter.clone(),
                efforts: efforts.clone(),
                tiers: tiers.clone(),
            });
        }
    }
    result
}

pub(super) fn metadata(models: &[Model], selected: &Model) -> MainSessionMetadata {
    MainSessionMetadata {
        models: models.iter().map(|m| m.display.clone()).collect(),
        efforts: selected.efforts.clone(),
        service_tier: selected.tier(),
        service_tiers: selected.tiers.clone(),
        modes: vec![
            json!({"id":"agent","name":"Agent"}),
            json!({"id":"plan","name":"Plan"}),
        ],
        ..Default::default()
    }
}

impl Model {
    pub(super) fn parameter(&self, key: &str) -> Option<String> {
        self.wire["params"]
            .as_array()?
            .iter()
            .find(|p| p["id"] == key)?["value"]
            .as_str()
            .map(str::to_owned)
    }
    pub(super) fn set_parameter(&mut self, key: &str, value: &str) {
        let params = self.wire["params"]
            .as_array_mut()
            .expect("model parameters");
        if let Some(param) = params.iter_mut().find(|p| p["id"] == key) {
            param["value"] = value.into();
        } else {
            params.push(json!({"id":key,"value":value}));
        }
    }
    pub(super) fn effort(&self) -> Option<String> {
        self.parameter(self.effort_parameter.as_deref()?)
    }
    pub(super) fn tier(&self) -> Option<String> {
        match self.parameter("fast").as_deref() {
            Some("true") => Some("priority".into()),
            Some("false") => Some("standard".into()),
            _ => None,
        }
    }
}

pub(super) fn restore(models: &[Model], saved: &Value) -> Result<Model, String> {
    let mut model = models
        .iter()
        .find(|m| {
            m.wire["id"] == saved["id"]
                && m.wire["params"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|p| {
                        p["id"] != "fast" && p["id"].as_str() != m.effort_parameter.as_deref()
                    })
                    .all(|p| {
                        saved["params"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|v| v == p)
                    })
        })
        .cloned()
        .ok_or("The saved Cursor SDK model is no longer available")?;
    for param in saved["params"].as_array().into_iter().flatten() {
        if let (Some(key), Some(value)) = (param["id"].as_str(), param["value"].as_str()) {
            model.set_parameter(key, value);
        }
    }
    Ok(model)
}

// The central catalog owns persistence and reuse; only this adapter interprets
// its model request data. Missing data marks old caches for a one-time refresh.
pub(super) fn from_catalog(catalog: &crate::ConfigurationCatalog) -> Option<Vec<Model>> {
    if catalog.models.is_empty() {
        return None;
    }
    catalog
        .models
        .iter()
        .map(|model| {
            if model.provider != crate::Backend::Cursor.as_str() {
                return None;
            }
            let data = model.adapter_data.as_ref()?;
            if data["version"] != 1
                || data["model"]["id"].as_str()?.is_empty()
                || !data["model"]["params"].is_array()
            {
                return None;
            }
            let effort_parameter = data["effortParameter"].as_str().map(str::to_owned);
            let efforts = model.efforts.clone().unwrap_or_default();
            if !efforts.is_empty() && effort_parameter.is_none() {
                return None;
            }
            Some(Model {
                id: model.id.clone(),
                wire: data["model"].clone(),
                display: serde_json::to_value(model).ok()?,
                effort_parameter,
                efforts,
                tiers: model.service_tiers.clone(),
            })
        })
        .collect()
}

pub(super) fn catalog(raw: &[Value]) -> Result<crate::ConfigurationCatalog, String> {
    let models = models(raw);
    let selected = models
        .first()
        .ok_or("Cursor SDK returned no usable models")?;
    super::super::configuration_catalog(metadata(&models, selected))
}

#[cfg(test)]
#[path = "configuration_tests.rs"]
mod tests;
