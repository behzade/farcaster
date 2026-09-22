use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    SessionOperation,
    extensions::{AgentMode, Model, PromptMode, SessionState, SlashCommand},
};

/// A response's operation is derived from its payload, not independently mutable.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionResponse {
    pub id: Option<String>,
    pub result: Result<SessionResponsePayload, SessionResponseError>,
}

impl SessionResponse {
    pub fn success(id: Option<String>, payload: SessionResponsePayload) -> Self {
        Self {
            id,
            result: Ok(payload),
        }
    }

    pub fn failure(id: Option<String>, operation: SessionOperation, message: String) -> Self {
        Self {
            id,
            result: Err(SessionResponseError {
                operation,
                message,
                kind: SessionResponseErrorKind::RejectedBeforeAcceptance,
            }),
        }
    }

    pub fn cancelled(id: String, operation: SessionOperation, message: String) -> Self {
        Self {
            id: Some(id),
            result: Err(SessionResponseError {
                operation,
                message,
                kind: SessionResponseErrorKind::Cancelled,
            }),
        }
    }

    pub fn prompt_delivery_unknown(id: String, mode: PromptMode, message: String) -> Self {
        Self {
            id: Some(id),
            result: Err(SessionResponseError {
                operation: SessionOperation::Prompt(mode),
                message,
                kind: SessionResponseErrorKind::DeliveryUnknown,
            }),
        }
    }

    pub fn operation(&self) -> SessionOperation {
        match &self.result {
            Ok(payload) => payload.operation(),
            Err(error) => error.operation,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct SessionResponseError {
    pub operation: SessionOperation,
    pub message: String,
    pub kind: SessionResponseErrorKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionResponseErrorKind {
    #[default]
    RejectedBeforeAcceptance,
    /// The transport abandoned the request during a lifecycle change, not a backend rejection.
    Cancelled,
    DeliveryUnknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptOutcome {
    Cancelled,
    Accepted,
    RejectedBeforeAcceptance,
    /// The request ended without a definitive receipt. Durable outbox recovery owns
    /// later disposition; the composer must release its in-memory submission.
    DeliveryUnknown,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SessionResponsePayload {
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
    pub const fn operation(&self) -> SessionOperation {
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
pub enum SessionHistory {
    Preserve,
    // Transcript message payloads remain open-ended until the activity/history
    // contract is migrated; the response envelope itself is validated.
    Replace {
        messages: Vec<Value>,
        prompt_deliveries: Option<farcaster_sessions::PromptDeliveryReconciliation>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionUsage {
    pub tokens: SessionUsageTokens,
    #[serde(alias = "cost", skip_serializing_if = "Option::is_none")]
    pub total_cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_usage: Option<SessionContextUsage>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionUsageTokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total_tokens: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionContextUsage {
    pub tokens: Option<u64>,
    pub context_window: u64,
    pub percent: Option<f64>,
}
