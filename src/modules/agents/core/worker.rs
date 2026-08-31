use std::path::PathBuf;

use serde_json::Value;

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TokenUsage {
    pub(crate) input: u64,
    pub(crate) output: u64,
    pub(crate) cache_read: u64,
    pub(crate) cache_write: u64,
}

impl TokenUsage {
    pub(crate) const fn total(self) -> u64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
    }

    pub(crate) const fn saturating_add(self, other: Self) -> Self {
        Self {
            input: self.input.saturating_add(other.input),
            output: self.output.saturating_add(other.output),
            cache_read: self.cache_read.saturating_add(other.cache_read),
            cache_write: self.cache_write.saturating_add(other.cache_write),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct WorkerUsage {
    pub(crate) turn: TokenUsage,
    pub(crate) session: TokenUsage,
    pub(crate) context_window: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WorkerActivity {
    TextDelta {
        content_index: usize,
        delta: String,
    },
    ThinkingDelta {
        content_index: usize,
        delta: String,
    },
    ToolStarted {
        id: String,
        name: String,
        args: Value,
    },
    ToolUpdated {
        id: String,
        content: Value,
    },
    ToolFinished {
        id: String,
        result: Value,
        is_error: bool,
    },
    Usage(WorkerUsage),
    CompactionStarted,
    CompactionFinished {
        aborted: bool,
        error: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WorkerEvent {
    Started,
    Settled { output: String },
    SessionChanged { locator: String },
    NeedsInput(WorkerInput),
    Activity(WorkerActivity),
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
    fn select_mode(&mut self, _mode: &str) -> Result<(), String> {
        Err("worker backend does not support mode selection".into())
    }
    fn poll(&mut self) -> Option<WorkerEvent>;
    fn close(&mut self) -> Result<(), String>;
}

pub(crate) trait WorkerSessionFactory: Send + Sync {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_usage_has_one_shared_saturating_accounting_rule() {
        let first = TokenUsage {
            input: 100,
            output: 20,
            cache_read: 80,
            cache_write: 10,
        };
        let total = first.saturating_add(TokenUsage {
            input: 50,
            output: 5,
            cache_read: 40,
            cache_write: 0,
        });
        assert_eq!(first.total(), 210);
        assert_eq!(total.input, 150);
        assert_eq!(total.output, 25);
        assert_eq!(total.cache_read, 120);
        assert_eq!(total.total(), 305);
    }
}
