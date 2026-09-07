use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) native: Option<Value>,
}
