mod pi;

use serde_json::Value;

use crate::protocol::{
    ExtensionUiRequest, ExtensionUiResponse, PromptImage, PromptMode, RpcResponse,
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BackendEvent {
    Response(RpcResponse),
    Interaction(ExtensionUiRequest),
    Activity(Value),
    Stderr(String),
    Failure(String),
}

pub(crate) trait SessionTransport {
    fn send(&mut self, request: BackendRequest) -> Result<String, String>;
    fn respond(&mut self, response: ExtensionUiResponse) -> Result<(), String>;
    fn poll(&mut self) -> Option<BackendEvent>;
    fn close(&mut self) -> Result<(), String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BackendRequest {
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
    Login {
        provider: Option<String>,
    },
    SelectModel {
        provider: String,
        model_id: String,
    },
    SelectReasoning {
        level: String,
    },
}

pub(crate) fn encode_pi_request(request: BackendRequest) -> serde_json::Value {
    pi::encode_request(request)
}

impl BackendRequest {
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
            Self::Login { .. } => "login",
            Self::SelectModel { .. } => "select model",
            Self::SelectReasoning { .. } => "select reasoning",
        }
    }
}
