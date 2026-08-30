use std::path::PathBuf;

use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WorkerContext {
    Fresh,
    Session(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerSendMode {
    Prompt,
    Queue,
    Steer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkerLaunch {
    pub(crate) project: PathBuf,
    pub(crate) context: WorkerContext,
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
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
