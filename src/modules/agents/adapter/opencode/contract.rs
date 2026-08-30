use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OpenCodeHttpMethod {
    Get,
    Post,
    Delete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenCodeHttpRequest {
    pub method: OpenCodeHttpMethod,
    pub path: String,
    pub body: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenCodeHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub(crate) trait OpenCodeHttpTransport {
    fn execute(&mut self, request: OpenCodeHttpRequest) -> Result<OpenCodeHttpResponse, String>;
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct OpenCodeLocation {
    pub directory: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenCodeSession {
    pub id: String,
    pub location: OpenCodeLocation,
    #[serde(default, rename = "parentID")]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum OpenCodeDelivery {
    Steer,
    Queue,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct OpenCodeFileInput {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenCodePromptAdmission {
    pub id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    pub delivery: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OpenCodeEvent {
    pub id: Option<String>,
    pub event: Option<String>,
    pub data: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct DataEnvelope<T> {
    pub data: T,
}

#[derive(Debug, Deserialize)]
pub(super) struct ErrorEnvelope {
    #[serde(rename = "_tag")]
    pub tag: String,
    pub message: String,
}
