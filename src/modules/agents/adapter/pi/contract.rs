use serde_json::Value;

use super::wire::PiResponse;
use crate::agents::extensions::{ExtensionUiRequest, ExtensionUiResponse, PromptImage, PromptMode};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PiEvent {
    Response(PiResponse),
    Interaction(ExtensionUiRequest),
    Activity(Value),
    Stderr(String),
    Failure(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PiRequest {
    ConfigureSteering,
    LoadState,
    LoadHistory,
    LoadUsage,
    ListModels,
    ListReasoningLevels,
    ListCommands,
    Prompt {
        mode: PromptMode,
        message: String,
        images: Vec<PromptImage>,
    },
    Abort,
    Compact {
        instructions: Option<String>,
    },
    ExportHtml {
        output_path: Option<String>,
    },
    Rename {
        name: String,
    },
    ForkAt {
        entry_id: String,
    },
    SelectModel {
        provider: String,
        model_id: String,
    },
    SelectReasoning {
        level: String,
    },
}

impl PiRequest {
    pub(crate) const fn operation(&self) -> &'static str {
        match self {
            Self::ConfigureSteering => "configure steering",
            Self::LoadState => "load state",
            Self::LoadHistory => "load history",
            Self::LoadUsage => "load usage",
            Self::ListModels => "list models",
            Self::ListReasoningLevels => "list reasoning levels",
            Self::ListCommands => "list commands",
            Self::Prompt { mode, .. } => match mode {
                PromptMode::Normal => "prompt",
                PromptMode::Steer => "steer",
                PromptMode::FollowUp => "follow up",
            },
            Self::Abort => "abort",
            Self::Compact { .. } => "compact",
            Self::ExportHtml { .. } => "export HTML",
            Self::Rename { .. } => "rename session",
            Self::ForkAt { .. } => "fork session",
            Self::SelectModel { .. } => "select model",
            Self::SelectReasoning { .. } => "select reasoning",
        }
    }
}

pub(crate) trait PiSessionTransport {
    fn send(&mut self, request: PiRequest) -> Result<String, String>;
    fn respond(&mut self, response: ExtensionUiResponse) -> Result<(), String>;
    fn poll(&mut self) -> Option<PiEvent>;
    fn close(&mut self) -> Result<(), String>;
}
