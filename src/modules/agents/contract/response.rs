use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    SessionOperation,
    extensions::{AgentMode, Model, PromptMode, SessionState, SlashCommand},
};

/// A response's operation is derived from its payload, not independently mutable.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SessionResponse {
    pub(crate) id: Option<String>,
    pub(crate) result: Result<SessionResponsePayload, SessionResponseError>,
}

impl SessionResponse {
    pub(crate) fn success(id: Option<String>, payload: SessionResponsePayload) -> Self {
        Self {
            id,
            result: Ok(payload),
        }
    }

    pub(crate) fn failure(
        id: Option<String>,
        operation: SessionOperation,
        message: String,
    ) -> Self {
        Self {
            id,
            result: Err(SessionResponseError { operation, message }),
        }
    }

    pub(crate) fn operation(&self) -> SessionOperation {
        match &self.result {
            Ok(payload) => payload.operation(),
            Err(error) => error.operation,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub(crate) struct SessionResponseError {
    pub(crate) operation: SessionOperation,
    pub(crate) message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SessionResponsePayload {
    ConfigureSteering,
    ApplySteering,
    LoadState(Box<SessionState>),
    LoadHistory(SessionHistory),
    LoadUsage(SessionUsage),
    ListModels(Vec<Model>),
    ListReasoningLevels(Vec<String>),
    ListModes {
        modes: Vec<AgentMode>,
        selected: Option<String>,
    },
    ListCommands(Vec<SlashCommand>),
    Prompt(PromptMode),
    Abort,
    Compact,
    ExportHtml {
        path: String,
    },
    Rename,
    ForkAt {
        cancelled: bool,
    },
    SelectModel(Model),
    SelectReasoning,
    SelectServiceTier,
    SelectMode,
    Other,
}

impl SessionResponsePayload {
    pub(crate) const fn operation(&self) -> SessionOperation {
        match self {
            Self::ConfigureSteering => SessionOperation::ConfigureSteering,
            Self::ApplySteering => SessionOperation::ApplySteering,
            Self::LoadState(_) => SessionOperation::LoadState,
            Self::LoadHistory(_) => SessionOperation::LoadHistory,
            Self::LoadUsage(_) => SessionOperation::LoadUsage,
            Self::ListModels(_) => SessionOperation::ListModels,
            Self::ListReasoningLevels(_) => SessionOperation::ListReasoningLevels,
            Self::ListModes { .. } => SessionOperation::ListModes,
            Self::ListCommands(_) => SessionOperation::ListCommands,
            Self::Prompt(mode) => SessionOperation::Prompt(*mode),
            Self::Abort => SessionOperation::Abort,
            Self::Compact => SessionOperation::Compact,
            Self::ExportHtml { .. } => SessionOperation::ExportHtml,
            Self::Rename => SessionOperation::Rename,
            Self::ForkAt { .. } => SessionOperation::ForkAt,
            Self::SelectModel(_) => SessionOperation::SelectModel,
            Self::SelectReasoning => SessionOperation::SelectReasoning,
            Self::SelectServiceTier => SessionOperation::SelectServiceTier,
            Self::SelectMode => SessionOperation::SelectMode,
            Self::Other => SessionOperation::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SessionHistory {
    Preserve,
    // Transcript message payloads remain open-ended until the activity/history
    // contract is migrated; the response envelope itself is validated.
    Replace(Vec<Value>),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionUsage {
    pub(crate) tokens: SessionUsageTokens,
    #[serde(alias = "cost", skip_serializing_if = "Option::is_none")]
    pub(crate) total_cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) context_usage: Option<SessionContextUsage>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionUsageTokens {
    pub(crate) input: u64,
    pub(crate) output: u64,
    pub(crate) cache_read: u64,
    pub(crate) cache_write: u64,
    pub(crate) total_tokens: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionContextUsage {
    pub(crate) tokens: Option<u64>,
    pub(crate) context_window: u64,
    pub(crate) percent: Option<f64>,
}
