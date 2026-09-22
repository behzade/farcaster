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
        {
            if value.is_empty() || value != value.trim() || value.chars().any(char::is_control) {
                return Err(format!(
                    "worker {name} must be nonempty, without surrounding whitespace or control characters"
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerProfile {
    pub name: String,
    pub description: String,
    pub models: Vec<WorkerExecution>,
}

impl WorkerProfile {
    pub fn new(name: String) -> Self {
        Self {
            name,
            description: String::new(),
            models: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerProfiles {
    pub profiles: Vec<WorkerProfile>,
}

impl WorkerProfiles {
    pub fn from_saved(value: serde_json::Value) -> Result<Self, String> {
        let mut profiles: Self = if value.get("profiles").is_some() {
            serde_json::from_value(value).map_err(|error| error.to_string())?
        } else {
            legacy::migrate(value)?
        };
        profiles.migrate_reserved_profile_names();
        profiles.migrate_deprecated_cursor_model_ids();
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

    pub fn validate(&self) -> Result<(), String> {
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
            if profile.models.is_empty() {
                return Err(format!("{}: add at least one model", profile.name));
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
        let execution = definition.models.iter().find(|model| available(model))
            .ok_or_else(|| format!("no available model for worker profile '{profile}'; configure one of its harnesses or edit its model list in Settings"))?;
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
