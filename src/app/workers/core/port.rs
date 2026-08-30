use std::path::PathBuf;

use super::super::{WorkerContext, WorkerInput, WorkerInputResponse};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerSendMode {
    Prompt,
    Queue,
    Steer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkerLaunch {
    pub(crate) project: PathBuf,
    pub(crate) parent_session: String,
    pub(crate) context: WorkerContext,
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WorkerEvent {
    Started,
    Settled { output: String },
    SessionChanged { locator: String },
    NeedsInput(WorkerInput),
    Failed(String),
}

pub(crate) trait WorkerSession: Send {
    fn send(&mut self, message: String, mode: WorkerSendMode) -> Result<(), String>;
    fn respond(&mut self, response: WorkerInputResponse) -> Result<(), String>;
    fn abort(&mut self) -> Result<(), String>;
    fn poll(&mut self) -> Option<WorkerEvent>;
    fn close(&mut self) -> Result<(), String>;
}

pub(crate) trait WorkerSessionFactory: Send + Sync {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String>;
}
