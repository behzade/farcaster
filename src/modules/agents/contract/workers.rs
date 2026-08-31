use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{WorkerContext, WorkerInput};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerMessageMode {
    #[default]
    Auto,
    Prompt,
    Steer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StartWorker {
    pub(crate) project: PathBuf,
    pub(crate) prompt: String,
    pub(crate) backend: String,
    pub(crate) parent_session: String,
    pub(crate) context: WorkerContext,
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerStatus {
    Running,
    Stopping,
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
