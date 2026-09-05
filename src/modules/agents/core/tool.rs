use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Backend-supplied facts, never inferred from shell source by the transcript.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolCategory {
    Read,
    Search,
    List,
    Change,
    Execute,
    Fetch,
    Delegate,
    #[serde(other)]
    Other,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ToolMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) category: Option<ToolCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) targets: Vec<String>,
    /// Native input/item retained for inspection, not interpreted above adapters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) native: Option<Value>,
}
