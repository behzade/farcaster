//! Decode review artifacts from successful tool results, not model prose or arguments.
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use crate::app::{
    reviews::Review,
    views::transcript::conversation::{ToolExecutionState, TranscriptItem},
};

#[derive(Clone, Deserialize)]
pub(super) struct Artifact {
    version: u32,
    pub(super) project: PathBuf,
    pub(super) review: Review,
}

pub(super) fn from_item(item: &TranscriptItem) -> Option<Artifact> {
    if item.tool_execution_state() != Some(ToolExecutionState::Succeeded) {
        return None;
    }
    let result = item.tool_details.as_ref()?.result.as_ref()?;
    find(result, 0, &mut 1000)
}

// MCP adapters retain either structuredContent or JSON in a text block. Read
// the result only, never tool arguments or arbitrary assistant Markdown.
fn find(value: &Value, depth: usize, budget: &mut usize) -> Option<Artifact> {
    if depth > 8 || *budget == 0 {
        return None;
    }
    *budget -= 1;
    if let Some(value) = value.get("farcaster_review") {
        let artifact: Artifact = serde_json::from_value(value.clone()).ok()?;
        if artifact.version == 1
            && artifact.project.is_absolute()
            && artifact.review.validate().is_ok()
        {
            return Some(artifact);
        }
        return None;
    }
    match value {
        Value::String(text) if text.len() <= 1024 * 1024 => {
            let parsed = serde_json::from_str::<Value>(text).ok()?;
            find(&parsed, depth + 1, budget)
        }
        Value::Array(values) => values
            .iter()
            .find_map(|value| find(value, depth + 1, budget)),
        Value::Object(values) => ["structuredContent", "content", "text", "result"]
            .iter()
            .filter_map(|key| values.get(*key))
            .find_map(|value| find(value, depth + 1, budget)),
        _ => None,
    }
}

#[cfg(test)]
#[path = "review_artifact_tests.rs"]
mod tests;
