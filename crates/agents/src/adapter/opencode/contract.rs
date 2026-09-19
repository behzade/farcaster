use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenCodeHttpMethod {
    Get,
    Post,
    Patch,
    Delete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenCodeHttpRequest {
    pub method: OpenCodeHttpMethod,
    pub path: String,
    pub body: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenCodeHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub trait OpenCodeHttpTransport {
    fn execute(&mut self, request: OpenCodeHttpRequest) -> Result<OpenCodeHttpResponse, String>;

    fn execute_prompt(
        &mut self,
        request: OpenCodeHttpRequest,
    ) -> Result<OpenCodeHttpResponse, OpenCodePromptDispatchError> {
        self.execute(request)
            .map_err(OpenCodePromptDispatchError::Unsent)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpenCodePromptDispatchError {
    Unsent(String),
    Unknown(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct OpenCodeLocation {
    pub directory: String,
    #[serde(
        default,
        rename = "workspaceID",
        skip_serializing_if = "Option::is_none"
    )]
    pub workspace_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeSession {
    pub id: String,
    pub location: OpenCodeLocation,
    #[serde(default, rename = "parentID")]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub model: Option<OpenCodeModelSelection>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct OpenCodeModelSelection {
    pub id: String,
    #[serde(rename = "providerID")]
    pub provider_id: String,
    #[serde(default, deserialize_with = "deserialize_variant")]
    pub variant: Option<String>,
}

fn deserialize_variant<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    // OpenCode persists an omitted selection as the reserved "default" marker.
    Ok(Option::<String>::deserialize(deserializer)?.filter(|variant| variant != "default"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenCodeDelivery {
    Steer,
    Queue,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OpenCodeFileInput {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodePromptAdmission {
    pub id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    pub delivery: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpenCodeEvent {
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
