#[cfg(test)]
#[path = "reviews_tests.rs"]
mod tests;

use rmcp::schemars;
use serde::Deserialize;

use crate::app::reviews::{Review, ReviewLocation, resolve_path};

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Params {
    /// Short review title (at most 200 bytes).
    title: String,
    /// 1–100 suggested locations. Not an exhaustive or verified changeset.
    items: Vec<Location>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Location {
    /// Project-relative file path, without traversal.
    path: String,
    /// Optional inclusive, 1-based start line.
    start_line: Option<u32>,
    /// Optional inclusive end line; requires start_line.
    end_line: Option<u32>,
    /// What to inspect here (at most 1000 bytes, single line).
    note: String,
}

pub(super) fn submit(
    caller: &crate::agents::CallerContext,
    params: Params,
) -> Result<serde_json::Value, String> {
    let review = Review {
        title: params.title,
        items: params
            .items
            .into_iter()
            .map(|item| ReviewLocation {
                path: item.path,
                start_line: item.start_line,
                end_line: item.end_line,
                note: item.note,
            })
            .collect(),
    };
    review.validate()?;
    for item in &review.items {
        resolve_path(&caller.project, &item.path)?;
    }
    // The normal tool-result transcript/history carries this artifact. No
    // editor side effect, separate UI event, or second persistence path.
    Ok(serde_json::json!({
        "farcaster_review": {
            "version": 1,
            "project": caller.project.canonicalize().map_err(|e| e.to_string())?,
            "review": review,
        }
    }))
}
