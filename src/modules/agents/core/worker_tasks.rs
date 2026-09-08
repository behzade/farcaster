use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerExecution {
    pub(crate) harness: String,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) effort: Option<String>,
}

impl WorkerExecution {
    pub(crate) fn validate(&self) -> Result<(), String> {
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
pub(crate) struct WorkerProfile {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) models: Vec<WorkerExecution>,
}

impl WorkerProfile {
    pub(crate) fn new(name: String) -> Self {
        Self {
            name,
            ..WorkerProfiles::default()
                .profiles
                .into_iter()
                .find(|profile| profile.name == "fast")
                .expect("bundled worker profiles must include fast")
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerProfiles {
    pub(crate) profiles: Vec<WorkerProfile>,
}

impl Default for WorkerProfiles {
    fn default() -> Self {
        let profiles: Self = toml::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/worker_profiles.toml"
        )))
        .expect("bundled worker profiles must be valid TOML");
        profiles
            .validate()
            .expect("bundled worker profiles must be valid");
        profiles
    }
}

impl WorkerProfiles {
    pub(crate) fn from_saved(value: serde_json::Value) -> Result<Self, String> {
        let profiles: Self = if value.get("profiles").is_some() {
            serde_json::from_value(value).map_err(|error| error.to_string())?
        } else {
            legacy::migrate(value)?
        };
        profiles.validate()?;
        Ok(profiles)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        let mut names = BTreeSet::new();
        for profile in &self.profiles {
            if !super::super::valid_worker_name(&profile.name) {
                return Err("profile names must be 1–48 ASCII letters, numbers, '-' or '_' and start with a letter or number".into());
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

    pub(crate) fn resolve(
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct WorkerAssignment {
    pub(crate) profile: String,
    pub(crate) execution: WorkerExecution,
}

#[cfg(test)]
#[path = "worker_tasks_tests.rs"]
mod tests;

#[path = "worker_profiles_legacy.rs"]
mod legacy;
