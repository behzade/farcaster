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

impl ToolMetadata {
    /// Malformed values never match; omitted fields use the deserializer's defaults.
    pub fn matches_value(&self, value: &Value) -> bool {
        let Self {
            category,
            title,
            targets,
            native,
        } = self;
        let (next_category, next_title, next_targets, next_native) = match value {
            Value::Object(fields) => (
                fields.get("category"),
                fields.get("title"),
                fields.get("targets"),
                fields.get("native"),
            ),
            Value::Array(fields) if fields.len() <= 4 => {
                (fields.first(), fields.get(1), fields.get(2), fields.get(3))
            }
            _ => return false,
        };
        let category_matches = match next_category {
            None | Some(Value::Null) => category.is_none(),
            Some(value) => {
                ToolCategory::deserialize(value).is_ok_and(|next| Some(next) == *category)
            }
        };
        let title_matches = match next_title {
            None | Some(Value::Null) => title.is_none(),
            Some(Value::String(next)) => title.as_ref() == Some(next),
            _ => false,
        };
        let targets_match = match next_targets {
            None => targets.is_empty(),
            Some(Value::Array(next)) => {
                next.len() == targets.len()
                    && next
                        .iter()
                        .zip(targets)
                        .all(|(next, target)| next.as_str() == Some(target.as_str()))
            }
            _ => false,
        };
        category_matches
            && title_matches
            && targets_match
            && next_native.filter(|value| !value.is_null()) == native.as_ref()
    }
}

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;
