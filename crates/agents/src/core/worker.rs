use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

use super::super::{
    AccountUsage, PeerMessage, SessionGoal, WorkerContext, WorkerInput, WorkerInputResponse,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerActivityState {
    Starting,
    Working,
    Idle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerSendMode {
    Prompt,
    Queue,
    Steer,
}

impl WorkerSendMode {
    pub const fn for_peer(activity: WorkerActivityState) -> Option<Self> {
        match activity {
            WorkerActivityState::Starting => None,
            WorkerActivityState::Working => Some(Self::Steer),
            WorkerActivityState::Idle => Some(Self::Prompt),
        }
    }
}

#[derive(Clone, Debug)]
pub struct WorkerLaunch {
    pub slot: Option<super::WorkerSlot>,
    pub worker_id: String,
    pub worker_name: String,
    pub project: PathBuf,
    pub parent_session: String,
    pub parent_worker_id: Option<String>,
    pub context: WorkerContext,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub service_tier: Option<String>,
    pub access_mode: crate::HarnessAccessMode,
    pub app_proxy: Option<String>,
    pub ephemeral: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl TokenUsage {
    pub const fn total(self) -> u64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
    }

    pub const fn saturating_add(self, other: Self) -> Self {
        Self {
            input: self.input.saturating_add(other.input),
            output: self.output.saturating_add(other.output),
            cache_read: self.cache_read.saturating_add(other.cache_read),
            cache_write: self.cache_write.saturating_add(other.cache_write),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorkerUsage {
    pub turn: TokenUsage,
    pub session: TokenUsage,
    pub context_window: u64,
    pub cost: Option<f64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolReviewState {
    Reviewing,
    Approved,
    Blocked,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChildSessionOutcome {
    Complete,
    Failed,
    Incomplete,
}

impl ChildSessionOutcome {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Incomplete => "incomplete",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkerActivity {
    InputDelivered {
        mode: WorkerSendMode,
        message: String,
    },
    InputDeliveredWithImages {
        mode: WorkerSendMode,
        message: String,
        images: Vec<crate::extensions::PromptImage>,
    },
    SubmittedInputDelivered {
        submission_id: String,
        mode: WorkerSendMode,
        message: String,
    },
    SubmittedInputDeliveredWithImages {
        submission_id: String,
        mode: WorkerSendMode,
        message: String,
        images: Vec<crate::extensions::PromptImage>,
    },
    PeerInputDelivered {
        message: PeerMessage,
    },
    TurnStarted,
    TextDelta {
        content_index: usize,
        delta: String,
    },
    ThinkingStarted {
        content_index: usize,
    },
    ThinkingDelta {
        content_index: usize,
        delta: String,
    },
    ToolStarted {
        id: String,
        name: String,
        args: Value,
        metadata: super::ToolMetadata,
    },
    ToolMetadataChanged {
        id: String,
        args: Option<Value>,
        metadata: super::ToolMetadata,
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
    ToolReviewChanged {
        id: String,
        state: ToolReviewState,
        detail: Option<String>,
    },
    Usage(WorkerUsage),
    SelectionChanged {
        model: Option<(String, String)>,
        effort: Option<Option<String>>,
    },
    CommandsChanged {
        commands: Vec<Value>,
    },
    ModeChanged(String),
    TitleChanged(String),
    ServiceTierChanged {
        selected: Option<String>,
        options: Vec<String>,
    },
    ConfigurationChanged {
        models: Vec<Value>,
        efforts: Vec<String>,
        modes: Vec<Value>,
        selected_model: Option<Value>,
        selected_effort: Option<String>,
    },
    ServiceStatusChanged {
        name: String,
        status: String,
        error: Option<Value>,
        failure_reason: Option<Value>,
    },
    AccountUsageChanged(AccountUsage),
    SessionGoalChanged(Option<SessionGoal>),
    ChildSessionsChanged {
        id: String,
        title: Option<String>,
        is_running: bool,
        outcome: Option<ChildSessionOutcome>,
        /// `None` inherits the owning session's selection; `Some` is authoritative.
        execution: Option<WorkerModelSelection>,
    },
    CompactionStarted,
    CompactionFinished {
        aborted: bool,
        error: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkerEvent {
    Started,
    Settled {
        output: String,
    },
    SessionChanged {
        locator: String,
    },
    NeedsInput(WorkerInput),
    Activity(WorkerActivity),
    RequestFailed {
        operation: String,
        error: String,
    },
    /// This submission has no definitive receipt. The connection may still be usable.
    PromptDeliveryUnknown {
        submission_id: String,
        error: String,
    },
    /// The backend definitively cancelled this submission before delivery.
    PromptCancelled {
        submission_id: String,
    },
    Failed(String),
}

pub trait WorkerSession: Send {
    fn tracks_prompt_delivery(&self, _mode: WorkerSendMode) -> bool {
        false
    }
    /// Whether this backend can definitively report that an admitted prompt
    /// was cancelled before delivery.
    fn can_cancel_prompt_before_delivery(&self, _mode: WorkerSendMode) -> bool {
        false
    }
    fn send(&mut self, message: String, mode: WorkerSendMode) -> Result<(), String>;
    fn send_peer_message(
        &mut self,
        message: &PeerMessage,
        mode: WorkerSendMode,
    ) -> Result<(), String> {
        self.send(message.prompt(), mode)
    }
    fn send_with_images(
        &mut self,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::extensions::PromptImage>,
    ) -> Result<(), String> {
        if images.is_empty() {
            self.send(message, mode)
        } else {
            Err("worker backend does not support image input".into())
        }
    }
    /// Return true only after the backend confirms admission. Async backends
    /// return false and later report the caller's id through poll_prompt_ack.
    fn submit_prompt(
        &mut self,
        _id: String,
        _message: String,
        _mode: WorkerSendMode,
        _images: Vec<crate::extensions::PromptImage>,
    ) -> Result<bool, String> {
        Err("worker backend does not implement prompt acknowledgements".into())
    }
    fn poll_prompt_ack(&mut self) -> Option<(String, Result<(), String>)> {
        None
    }
    fn respond(&mut self, response: WorkerInputResponse) -> Result<(), String>;
    fn abort(&mut self) -> Result<(), String>;
    /// Apply submitted steering without discarding pending input.
    /// Backends that deliver steering on submission need no extra action.
    fn apply_steering(&mut self) -> Result<(), String> {
        Ok(())
    }
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
    fn reset_effort(&mut self) -> Result<(), String> {
        Err("worker backend does not support effort reset".into())
    }
    fn model_selection(&self) -> Option<WorkerModelSelection> {
        None
    }
    fn select_service_tier(&mut self, _tier: &str) -> Result<(), String> {
        Err("worker backend does not support service tier selection".into())
    }
    fn select_mode(&mut self, _mode: &str) -> Result<(), String> {
        Err("worker backend does not support mode selection".into())
    }
    fn poll(&mut self) -> Option<WorkerEvent>;
    fn close(&mut self) -> Result<(), String>;
}

/// An authoritative selection, including an explicitly unset effort.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkerModelSelection {
    pub model: Option<(String, String)>,
    pub effort: Option<String>,
}

pub trait WorkerSessionFactory: Send + Sync {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String>;
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
