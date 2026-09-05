use std::sync::Arc;

use serde_json::Value;

use super::{TranscriptItem, display_tool_name};
use crate::agents::{CommonTool, ToolCategory, ToolMetadata};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ToolExecutionState {
    #[default]
    Pending,
    Running,
    Succeeded,
    Failed,
}

/// Retain structured data independently of the readable input/output preview.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolDetails {
    pub name: String,
    pub arguments: Value,
    pub result: Option<Value>,
    pub metadata: ToolMetadata,
    pub state: ToolExecutionState,
}

impl ToolDetails {
    pub(super) fn from_call(
        name: &str,
        arguments: Option<&Value>,
        metadata: Option<&Value>,
    ) -> Self {
        let arguments = arguments.cloned().unwrap_or(Value::Null);
        let mut metadata: ToolMetadata = metadata
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        // Legacy canonical histories can lack metadata. Only shared tool names
        // are recognized here; native aliases and shell source belong to adapters.
        if metadata.category.is_none() {
            metadata.category = CommonTool::from_name(name).map(|tool| match tool {
                CommonTool::Read => ToolCategory::Read,
                CommonTool::Write | CommonTool::Edit => ToolCategory::Change,
                CommonTool::Bash => ToolCategory::Execute,
            });
        }
        if metadata.targets.is_empty()
            && matches!(
                metadata.category,
                Some(ToolCategory::Read | ToolCategory::Change)
            )
            && let Some(path) = arguments.get("path").and_then(Value::as_str)
            && !path.is_empty()
        {
            metadata.targets.push(path.to_owned());
        }
        Self {
            name: name.to_owned(),
            arguments,
            result: None,
            metadata,
            state: ToolExecutionState::Pending,
        }
    }

    pub(crate) fn summary(&self) -> String {
        if let Some(title) = self
            .metadata
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
        {
            return short_summary(title);
        }
        let action = match self.metadata.category {
            Some(ToolCategory::Read) => "Read",
            Some(ToolCategory::Search) => "Search",
            Some(ToolCategory::List) => "List files",
            Some(ToolCategory::Change) => "Change",
            Some(ToolCategory::Execute) => "Run command",
            Some(ToolCategory::Fetch) => "Fetch",
            Some(ToolCategory::Delegate) => "Agent task",
            Some(ToolCategory::Other) | None => {
                return short_summary(&display_tool_name(&self.name));
            }
        };
        if self.metadata.category == Some(ToolCategory::Execute) {
            return action.into();
        }
        match self.metadata.targets.as_slice() {
            [] => action.into(),
            [target] => short_summary(&format!("{action} {target}")),
            targets => format!("{action} {} targets", targets.len()),
        }
    }

    pub(crate) fn inspection_text(&self) -> String {
        let mut text = format!(
            "Tool: {}\n\nArguments:\n{}",
            self.name,
            pretty_json(&self.arguments)
        );
        if let Some(result) = &self.result {
            text.push_str(&format!("\n\nResult:\n{}", pretty_json(result)));
        }
        if let Some(native) = &self.metadata.native {
            text.push_str(&format!("\n\nNative data:\n{}", pretty_json(native)));
        }
        text
    }
}

impl TranscriptItem {
    /// Resolve legacy row flags and structured lifecycle in one place.
    pub(crate) fn tool_execution_state(&self) -> Option<ToolExecutionState> {
        if self.is_error {
            Some(ToolExecutionState::Failed)
        } else if self.streaming {
            Some(ToolExecutionState::Running)
        } else if let Some(details) = &self.tool_details {
            Some(details.state)
        } else {
            (!self.tool_output.is_empty()).then_some(ToolExecutionState::Succeeded)
        }
    }

    pub(super) fn finish_tool(&mut self, is_error: bool) {
        self.streaming = false;
        self.is_error = is_error;
        if let Some(details) = self.tool_details.as_mut().map(Arc::make_mut) {
            details.state = if is_error {
                ToolExecutionState::Failed
            } else {
                ToolExecutionState::Succeeded
            };
        }
    }
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn short_summary(text: &str) -> String {
    let mut summary = text
        .chars()
        .take_while(|ch| *ch != '\n')
        .take(96)
        .collect::<String>();
    if summary.len() < text.len() {
        summary.push('…');
    }
    summary
}
