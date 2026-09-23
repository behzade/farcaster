use crate::Backend;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerExecution {
    pub harness: Backend,
    pub provider: String,
    pub model: String,
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
}

impl WorkerExecution {
    pub fn validate(&self) -> Result<(), String> {
        let fields = [
            ("harness", self.harness.as_str()),
            ("provider", self.provider.as_str()),
            ("model", self.model.as_str()),
        ];
        for (name, value) in fields
            .into_iter()
            .chain(self.effort.as_deref().map(|value| ("effort", value)))
            .chain(
                self.service_tier
                    .as_deref()
                    .map(|value| ("service tier", value)),
            )
        {
            if value.is_empty() || value != value.trim() || value.chars().any(char::is_control) {
                return Err(format!(
                    "worker {name} must be nonempty, without surrounding whitespace or control characters"
                ));
            }
        }
        if self.service_tier.is_some() && self.harness != Backend::Cursor {
            return Err("service tier is available only for Cursor workers".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerProfile {
    pub name: String,
    pub description: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub models: Vec<WorkerExecution>,
}

const fn default_limit() -> usize {
    10
}
const fn default_enabled() -> bool {
    true
}

const BUILT_INS: [(&str, &str, usize); 4] = [
    (
        "smartest",
        "Use for the hardest tasks that need the strongest reasoning.",
        1,
    ),
    (
        "smart",
        "Use for hard tasks that benefit from stronger reasoning.",
        3,
    ),
    ("standard", "Use for most delegated work.", 10),
    ("light", "Use for small, parallel tasks.", 20),
];

impl WorkerProfile {
    pub fn new(name: String) -> Self {
        Self {
            name,
            description: String::new(),
            limit: default_limit(),
            enabled: true,
            models: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerProfiles {
    pub profiles: Vec<WorkerProfile>,
    #[serde(default = "default_limit")]
    pub inherit_limit: usize,
    #[serde(default = "default_enabled")]
    pub inherit_enabled: bool,
}

impl Default for WorkerProfiles {
    fn default() -> Self {
        Self {
            profiles: BUILT_INS
                .iter()
                .map(|(name, description, limit)| WorkerProfile {
                    name: (*name).into(),
                    description: (*description).into(),
                    limit: *limit,
                    enabled: true,
                    models: Vec::new(),
                })
                .collect(),
            inherit_limit: default_limit(),
            inherit_enabled: true,
        }
    }
}

impl WorkerProfiles {
    pub fn from_saved(mut value: serde_json::Value) -> Result<Self, String> {
        if let Some(profiles) = value
            .get_mut("profiles")
            .and_then(serde_json::Value::as_array_mut)
        {
            for profile in profiles {
                let Some(record) = profile.as_object_mut() else {
                    continue;
                };
                if !record.contains_key("limit") {
                    let limit = record
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|name| BUILT_INS.iter().find(|(builtin, _, _)| *builtin == name))
                        .map_or(default_limit(), |(_, _, limit)| *limit);
                    record.insert("limit".into(), limit.into());
                }
            }
        }
        let mut profiles: Self = if value.get("profiles").is_some() {
            serde_json::from_value(value).map_err(|error| error.to_string())?
        } else {
            legacy::migrate(value)?
        };
        profiles.migrate_reserved_profile_names();
        profiles.migrate_deprecated_cursor_model_ids();
        profiles.migrate_ordered_routes();
        profiles.ensure_built_ins();
        profiles.validate()?;
        Ok(profiles)
    }

    fn migrate_reserved_profile_names(&mut self) {
        let mut names = self
            .profiles
            .iter()
            .filter(|profile| !profile.name.eq_ignore_ascii_case("inherit"))
            .map(|profile| profile.name.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        for profile in &mut self.profiles {
            if !profile.name.eq_ignore_ascii_case("inherit") {
                continue;
            }
            let mut candidate = "inherit_custom".to_owned();
            let mut counter = 1;
            while names.contains(&candidate) {
                candidate = format!("inherit_custom_{counter}");
                counter += 1;
            }
            names.insert(candidate.clone());
            profile.name = candidate;
        }
    }

    fn migrate_deprecated_cursor_model_ids(&mut self) {
        for profile in &mut self.profiles {
            for execution in &mut profile.models {
                if execution.harness != Backend::Cursor
                    || execution.provider != Backend::Cursor.as_str()
                {
                    continue;
                }
                execution.model = match execution.model.as_str() {
                    "grok-4.6[effort=high,fast=true]" => "grok-4.6".into(),
                    "composer-2.5[fast=true]" => "composer-2.5".into(),
                    _ => continue,
                };
            }
        }
    }

    fn ensure_built_ins(&mut self) {
        for (name, description, limit) in BUILT_INS {
            if !self
                .profiles
                .iter()
                .any(|profile| profile.name.eq_ignore_ascii_case(name))
            {
                self.profiles.push(WorkerProfile {
                    name: name.into(),
                    description: description.into(),
                    limit,
                    enabled: true,
                    models: Vec::new(),
                });
            }
        }
    }

    fn migrate_ordered_routes(&mut self) {
        let mut names = self
            .profiles
            .iter()
            .map(|profile| profile.name.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        let mut extra = Vec::new();
        for profile in &mut self.profiles {
            if profile.models.len() <= 1 {
                continue;
            }
            for model in profile.models.drain(1..) {
                let mut index = 2;
                let name = loop {
                    let suffix = format!("_{index}");
                    let candidate = format!(
                        "{}{}",
                        &profile.name[..profile.name.len().min(48 - suffix.len())],
                        suffix
                    );
                    if names.insert(candidate.to_ascii_lowercase()) {
                        break candidate;
                    }
                    index += 1;
                };
                extra.push(WorkerProfile {
                    name,
                    description: profile.description.clone(),
                    limit: profile.limit,
                    enabled: profile.enabled,
                    models: vec![model],
                });
            }
        }
        self.profiles.extend(extra);
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.inherit_limit == 0 {
            return Err("inherit: worker limit must be positive".into());
        }
        let mut names = BTreeSet::new();
        for profile in &self.profiles {
            if !super::super::valid_worker_name(&profile.name) {
                return Err("profile names must be 1–48 ASCII letters, numbers, '-' or '_' and start with a letter or number".into());
            }
            if profile.name.eq_ignore_ascii_case("inherit") {
                return Err(
                    "'inherit' is reserved and cannot be used as a worker profile name".into(),
                );
            }
            if !names.insert(profile.name.to_ascii_lowercase()) {
                return Err(format!("duplicate worker profile: {}", profile.name));
            }
            if profile.description.trim().is_empty()
                || profile.description.chars().any(char::is_control)
            {
                return Err(format!(
                    "{}: provide a short description without control characters",
                    profile.name
                ));
            }
            if profile.limit == 0 {
                return Err(format!("{}: worker limit must be positive", profile.name));
            }
            if profile.models.len() > 1 {
                return Err(format!("{}: select one model", profile.name));
            }
            for model in &profile.models {
                model
                    .validate()
                    .map_err(|error| format!("{}: {error}", profile.name))?;
            }
        }
        Ok(())
    }

    pub fn resolve(
        &self,
        profile: &str,
        available: impl Fn(&WorkerExecution) -> bool,
    ) -> Result<WorkerAssignment, String> {
        self.validate()?;
        let definition = self
            .profiles
            .iter()
            .find(|definition| definition.name == profile)
            .ok_or_else(|| {
                format!("unknown worker profile: {profile}; refresh the tool schema for configured profiles")
            })?;
        if !definition.enabled {
            return Err(format!("worker profile '{profile}' is disabled"));
        }
        let execution = definition
            .models
            .first()
            .ok_or_else(|| format!("worker profile '{profile}' has no selected model"))?;
        if !available(execution) {
            return Err(format!(
                "selected model for worker profile '{profile}' is unavailable"
            ));
        }
        Ok(WorkerAssignment {
            profile: profile.into(),
            execution: execution.clone(),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkerAssignment {
    pub profile: String,
    pub execution: WorkerExecution,
}

#[cfg(test)]
#[path = "worker_tasks_tests.rs"]
mod tests;

#[path = "worker_profiles_legacy.rs"]
mod legacy;
