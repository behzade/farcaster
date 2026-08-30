use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub(crate) enum WorkerContext {
    #[default]
    Fresh,
    Session {
        session_locator: String,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerMessageMode {
    #[default]
    Auto,
    Prompt,
    Steer,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkerInput {
    pub(crate) id: String,
    pub(crate) prompt: String,
    pub(crate) options: Vec<String>,
    pub(crate) secret: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkerInputResponse {
    pub(crate) id: String,
    pub(crate) value: Option<String>,
    pub(crate) cancel: bool,
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
