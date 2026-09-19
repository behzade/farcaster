use std::path::PathBuf;

use farcaster_contracts::Backend;

use super::WorkerContext;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PeerMessage {
    pub(crate) from: String,
    pub(crate) message: String,
}

impl PeerMessage {
    const PROMPT_PREFIX: &'static str = "Message from Farcaster worker ";
    const LEGACY_PROMPT_PREFIX: &'static str = "Message from Farcaster peer ";

    pub(crate) fn prompt(&self) -> String {
        format!("{}{}:\n\n{}", Self::PROMPT_PREFIX, self.from, self.message)
    }

    pub(crate) fn from_prompt(prompt: &str) -> Option<Self> {
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

pub(crate) fn valid_worker_name(name: &str) -> bool {
    (1..=48).contains(&name.len())
        && name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'-' | b'_'))
        })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StartWorker {
    pub(crate) project: PathBuf,
    pub(crate) name: String,
    pub(crate) prompt: String,
    pub(crate) backend: Backend,
    pub(crate) parent_session: String,
    pub(crate) parent_worker_id: Option<String>,
    pub(crate) context: WorkerContext,
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) access_mode: super::HarnessAccessMode,
}

pub(crate) fn validate_child_access(
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
