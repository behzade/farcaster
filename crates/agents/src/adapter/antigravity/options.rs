use serde_json::{Value, json};
use std::path::PathBuf;

pub(in crate::adapter) struct SessionOptions {
    directories: Vec<PathBuf>,
    instructions: Option<String>,
}

impl SessionOptions {
    pub fn from_environment() -> Result<Self, String> {
        let directories: Vec<_> = std::env::var_os("FARCASTER_ANTIGRAVITY_ADD_DIRS")
            .iter()
            .flat_map(std::env::split_paths)
            .filter(|path| !path.as_os_str().is_empty())
            .collect();
        for path in &directories {
            if !path.is_absolute() || !path.is_dir() {
                return Err(format!(
                    "Additional Antigravity workspace folder must be an existing absolute directory: {}",
                    path.display()
                ));
            }
        }
        Ok(Self {
            directories,
            instructions: std::env::var("FARCASTER_ANTIGRAVITY_SYSTEM_PROMPT_APPEND")
                .ok()
                .filter(|text| !text.is_empty()),
        })
    }
    pub fn apply(&self, params: &mut Value, init: &Value) -> Result<(), String> {
        if !self.directories.is_empty() {
            if !init
                .pointer("/agentCapabilities/sessionCapabilities/additionalDirectories")
                .is_some_and(Value::is_object)
            {
                return Err(
                    "This Antigravity version does not support additional workspace folders".into(),
                );
            }
            params["additionalDirectories"] = json!(self.directories);
        }
        if let Some(text) = &self.instructions {
            if init
                .pointer("/agentCapabilities/_meta/antigravity/systemPrompt/append")
                .and_then(Value::as_bool)
                != Some(true)
            {
                return Err(
                    "This Antigravity version does not support appended instructions".into(),
                );
            }
            params["_meta"]["antigravity"]["systemPrompt"] = json!({"append":text});
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "options_tests.rs"]
mod tests;
