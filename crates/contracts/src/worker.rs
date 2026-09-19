use std::path::PathBuf;

use serde::Serialize;

use crate::Backend;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerInput {
    pub id: String,
    pub prompt: String,
    pub options: Vec<String>,
    pub secret: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    Pending,
    Running,
    Idle,
    NeedsInput,
    Failed,
    Stopped,
}

impl WorkerStatus {
    pub const fn terminal(self) -> bool {
        matches!(self, Self::Failed | Self::Stopped)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerSnapshot {
    pub id: String,
    pub backend: Backend,
    pub project: PathBuf,
    pub session_locator: Option<String>,
    pub status: WorkerStatus,
    pub output: Option<String>,
    pub error: Option<String>,
    pub pending_input: Option<WorkerInput>,
}
