use std::{
    path::{Path, PathBuf},
    sync::RwLock,
};

use serde::{Deserialize, Serialize};

use crate::Backend;

/// A named command that speaks one of Farcaster's supported backend protocols.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HarnessProfile {
    pub id: String,
    pub name: String,
    pub backend: Backend,
    pub executable: PathBuf,
    #[serde(default)]
    pub data_directory: Option<PathBuf>,
}

impl HarnessProfile {
    pub fn validate(&self) -> Result<(), String> {
        uuid::Uuid::parse_str(&self.id)
            .map_err(|_| "harness profile ID must be a UUID".to_owned())?;
        if self.name.trim().is_empty() || self.name.chars().any(char::is_control) {
            return Err(
                "harness profile name must not be empty or contain control characters".into(),
            );
        }
        if self.executable.as_os_str().is_empty()
            || (!self.executable.is_absolute() && self.executable.components().count() != 1)
        {
            return Err(
                "harness profile executable must be a command name or absolute path".into(),
            );
        }
        if let Some(directory) = &self.data_directory {
            if !directory.is_absolute() {
                return Err("harness profile data directory must be an absolute path".into());
            }
            if crate::profile_data_environment_key(self.backend).is_none() {
                return Err(format!(
                    "{} does not support a profile data directory",
                    self.backend
                ));
            }
        }
        Ok(())
    }

    pub fn data_environment_key(&self) -> Option<&'static str> {
        self.data_directory.as_ref()?;
        crate::profile_data_environment_key(self.backend)
    }

    pub fn is_selectable(&self) -> bool {
        !self.executable.is_absolute() || self.executable.is_file()
    }
}

#[derive(Default)]
pub struct HarnessProfiles {
    profiles: RwLock<Vec<HarnessProfile>>,
}

impl HarnessProfiles {
    pub fn list(&self) -> Result<Vec<HarnessProfile>, String> {
        self.profiles
            .read()
            .map(|profiles| profiles.clone())
            .map_err(|_| "harness profiles are unavailable".into())
    }

    pub fn get(&self, id: &str) -> Result<HarnessProfile, String> {
        self.list()?
            .into_iter()
            .find(|profile| profile.id == id)
            .ok_or_else(|| format!("unknown harness profile: {id}"))
    }

    pub fn replace(&self, profiles: Vec<HarnessProfile>) -> Result<(), String> {
        let mut current = self
            .profiles
            .write()
            .map_err(|_| "harness profiles are unavailable".to_owned())?;
        *current = profiles;
        Ok(())
    }
}

pub fn profile_id_from_locator(path: &Path) -> Option<String> {
    let backend = path.parent()?;
    let id = backend.parent()?;
    (id.parent()?.file_name()? == "profiles")
        .then(|| id.file_name()?.to_str())
        .flatten()
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
        .map(str::to_owned)
}

#[cfg(test)]
#[path = "profiles_tests.rs"]
mod tests;
