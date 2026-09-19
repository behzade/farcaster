//! Decode review artifacts from successful tool results, not model prose or arguments.
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    conversation::{ToolExecutionState, TranscriptItem, TranscriptKind},
    reviews::{Review, ReviewLocation},
};

#[derive(Clone, Deserialize)]
pub struct Artifact {
    version: u32,
    #[serde(default)]
    #[allow(dead_code)] // the transcript card renders from the review itself
    pub id: Option<String>,
    pub project: PathBuf,
    pub review: Review,
}

pub fn from_item(item: &TranscriptItem) -> Option<Artifact> {
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

/// Rebuild a review artifact from a submit_review row's own arguments when
/// history replay dropped the echoing result. The row itself is the durable
/// carrier of the review; rendering never depends on a separately stored card.
pub fn hydration_result(item: &TranscriptItem, project: &Path) -> Option<Value> {
    if item.kind != TranscriptKind::Tool
        || item.tool_execution_state() != Some(ToolExecutionState::Succeeded)
        || from_item(item).is_some()
    {
        return None;
    }
    let details = item.tool_details.as_ref()?;
    if !details.name.contains("submit_review") {
        return None;
    }
    let Some(review) = review_from_arguments(&details.arguments) else {
        let keys: Vec<_> = details
            .arguments
            .as_object()
            .map(|object| object.keys().collect())
            .unwrap_or_default();
        zlog::info!("review hydration: arguments did not decode; top-level keys={keys:?}");
        return None;
    };
    Some(json!({
        "farcaster_review": {"version": 1, "project": project, "review": review}
    }))
}

fn review_from_arguments(arguments: &Value) -> Option<Review> {
    // Farcaster MCP proxies may wrap the parameters one level deep: Cursor
    // nests them under "args", and completed tool updates can re-wrap them
    // under "arguments" next to a "prompt" summary.
    [
        Some(arguments),
        arguments.get("args"),
        arguments.get("arguments"),
    ]
    .into_iter()
    .flatten()
    .find_map(|candidate| {
        let title = candidate.get("title").and_then(Value::as_str)?;
        let items = candidate
            .get("items")?
            .as_array()?
            .iter()
            .filter_map(|location| {
                let path = location.get("path")?.as_str()?.to_owned();
                let line = |key: &str| {
                    location
                        .get(key)
                        .and_then(Value::as_u64)
                        .and_then(|line| u32::try_from(line).ok())
                };
                Some(ReviewLocation {
                    path,
                    start_line: line("start_line"),
                    end_line: line("end_line"),
                    note: location
                        .get("note")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                })
            })
            .collect::<Vec<_>>();
        let review = Review {
            title: title.to_owned(),
            items,
        };
        review.validate().ok()?;
        Some(review)
    })
}

#[cfg(test)]
#[path = "artifact_tests.rs"]
mod tests;
