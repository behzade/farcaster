use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommonTool {
    Read,
    Write,
    Edit,
    Bash,
}

impl CommonTool {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Edit => "edit",
            Self::Bash => "bash",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.trim();
        [Self::Read, Self::Write, Self::Edit, Self::Bash]
            .into_iter()
            .find(|tool| name.eq_ignore_ascii_case(tool.name()))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
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
pub struct ToolMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<ToolCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<Value>,
}
