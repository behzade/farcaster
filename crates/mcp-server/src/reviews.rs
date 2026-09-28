#[cfg(test)]
#[path = "reviews_tests.rs"]
mod tests;

use rmcp::schemars;
use serde::Deserialize;

use crate::review_domain::{Review, ReviewLocation, resolve_path};

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Params {
    title: String,
    items: Vec<Location>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Location {
    path: String,
    start_line: Option<u32>,
    end_line: Option<u32>,
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
    Ok(serde_json::json!({
        "farcaster_review": {
            "version": 1,
            "id": uuid::Uuid::new_v4().to_string(),
            "project": caller.project.canonicalize().map_err(|e| e.to_string())?,
            "review": review,
        }
    }))
}
