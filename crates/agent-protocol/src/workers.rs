use std::path::PathBuf;

use farcaster_contracts::Backend;

use super::WorkerContext;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerMessage {
    pub from: String,
    pub message: String,
}

impl PeerMessage {
    const PROMPT_PREFIX: &'static str = "Message from Farcaster worker ";
    const LEGACY_PROMPT_PREFIX: &'static str = "Message from Farcaster peer ";

    pub fn prompt(&self) -> String {
        format!("{}{}:\n\n{}", Self::PROMPT_PREFIX, self.from, self.message)
    }

    pub fn from_prompt(prompt: &str) -> Option<Self> {
        let (heading, message) = prompt.split_once("\n\n")?;
        let from = [Self::PROMPT_PREFIX, Self::LEGACY_PROMPT_PREFIX]
            .into_iter()
            .find_map(|prefix| heading.strip_prefix(prefix))?
            .strip_suffix(':')?;
        if !valid_worker_name(from) {
            return None;
        }
        Some(Self {
            from: from.to_owned(),
            message: message.to_owned(),
        })
    }
}

pub fn valid_worker_name(name: &str) -> bool {
    (1..=48).contains(&name.len())
        && name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'-' | b'_'))
        })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartWorker {
    pub project: PathBuf,
    pub name: String,
    pub prompt: String,
    pub backend: Backend,
    pub parent_session: String,
    pub parent_worker_id: Option<String>,
    pub context: WorkerContext,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub access_mode: super::HarnessAccessMode,
}

pub fn validate_child_access(
    parent: super::HarnessAccessMode,
    child: super::HarnessAccessMode,
) -> Result<(), String> {
    if parent != super::HarnessAccessMode::Full && child == super::HarnessAccessMode::Full {
        return Err("restricted parent cannot reuse an unrestricted child".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "workers_tests.rs"]
mod tests;
