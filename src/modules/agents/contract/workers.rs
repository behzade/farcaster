use std::path::PathBuf;

use serde::Serialize;

use super::{WorkerContext, WorkerInput};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PeerMessage {
    pub(crate) from: String,
    pub(crate) message: String,
}

impl PeerMessage {
    // Current backend protocols expose only user-input channels. Keep this
    // fallback envelope reversible so Farcaster retains worker presentation
    // when loading backend-owned history.
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
    pub(crate) backend: String,
    pub(crate) parent_session: String,
    pub(crate) parent_worker_id: Option<String>,
    pub(crate) context: WorkerContext,
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerStatus {
    Running,
    Idle,
    NeedsInput,
    Failed,
    Stopped,
}

impl WorkerStatus {
    pub(crate) const fn terminal(self) -> bool {
        matches!(self, Self::Failed | Self::Stopped)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkerSnapshot {
    pub(crate) id: String,
    pub(crate) backend: String,
    pub(crate) project: PathBuf,
    pub(crate) session_locator: Option<String>,
    pub(crate) status: WorkerStatus,
    pub(crate) output: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) pending_input: Option<WorkerInput>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_prompt_round_trips_structured_origin() {
        let peer = PeerMessage {
            from: "diff-review".into(),
            message: "review complete\nwith details".into(),
        };
        assert_eq!(PeerMessage::from_prompt(&peer.prompt()), Some(peer));
        assert!(PeerMessage::from_prompt("Message from Farcaster worker bad id:\n\nno").is_none());
        assert!(PeerMessage::from_prompt("ordinary user message").is_none());
        assert_eq!(
            PeerMessage::from_prompt("Message from Farcaster peer worker-7:\n\nlegacy")
                .map(|message| message.from),
            Some("worker-7".into())
        );
    }
}
