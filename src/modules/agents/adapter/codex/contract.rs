use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum CodexRequestId {
    Number(i64),
    String(String),
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct CodexRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CodexInbound {
    Response {
        id: CodexRequestId,
        result: Value,
    },
    Error {
        id: CodexRequestId,
        error: CodexRpcError,
    },
    Notification {
        method: String,
        params: Value,
    },
    ServerRequest {
        id: CodexRequestId,
        method: String,
        params: Value,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexClientInfo {
    pub name: String,
    pub title: Option<String>,
    pub version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexInitializeResponse {
    pub user_agent: String,
    pub codex_home: String,
    pub platform_family: String,
    pub platform_os: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexThread {
    pub id: String,
    #[serde(default)]
    pub parent_thread_id: Option<String>,
    pub cwd: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct CodexTurn {
    pub id: String,
    pub status: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type")]
pub(crate) enum CodexUserInput {
    #[serde(rename = "text")]
    Text {
        text: String,
        text_elements: Vec<Value>,
    },
    #[serde(rename = "image")]
    Image { url: String },
    #[serde(rename = "localImage")]
    LocalImage { path: String },
}

impl CodexUserInput {
    pub(crate) fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            text_elements: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ThreadResponse {
    pub thread: CodexThread,
}

#[derive(Debug, Deserialize)]
pub(super) struct TurnResponse {
    pub turn: CodexTurn,
}
