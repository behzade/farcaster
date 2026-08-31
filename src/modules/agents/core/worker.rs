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
    Activity(serde_json::Value),
    Failed(String),
}

pub(crate) trait WorkerSession: Send {
    fn send(&mut self, message: String, mode: WorkerSendMode) -> Result<(), String>;
    fn send_with_images(
        &mut self,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<(), String> {
        if images.is_empty() {
            self.send(message, mode)
        } else {
            Err("worker backend does not support image input".into())
        }
    }
    fn respond(&mut self, response: WorkerInputResponse) -> Result<(), String>;
    fn abort(&mut self) -> Result<(), String>;
    fn compact(&mut self) -> Result<(), String> {
        Err("worker backend does not support compaction".into())
    }
    fn rename(&mut self, _name: &str) -> Result<(), String> {
        Err("worker backend does not support session naming".into())
    }
    fn select_model(&mut self, _provider: &str, _model: &str) -> Result<(), String> {
        Err("worker backend does not support model selection".into())
    }
    fn select_effort(&mut self, _effort: &str) -> Result<(), String> {
        Err("worker backend does not support effort selection".into())
    }
    fn poll(&mut self) -> Option<WorkerEvent>;
    fn close(&mut self) -> Result<(), String>;
}

pub(crate) trait WorkerSessionFactory: Send + Sync {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String>;
}
