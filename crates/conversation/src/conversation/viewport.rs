use super::*;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct ToolPreview {
    fingerprint: [u8; 32],
    pub(super) summary: String,
}

fn fingerprint(item: &TranscriptItem) -> [u8; 32] {
    Sha256::digest(serde_json::to_vec(item).expect("transcript item is JSON serializable")).into()
}

impl TranscriptItem {
    /// Reduce the initial, collapsed tool row. The caller excludes review artifacts.
    pub fn for_collapsed_viewport(&self) -> Self {
        use farcaster_agent_protocol::ToolCategory;
        let Some(details) = &self.tool_details else {
            return self.clone();
        };
        if self.kind != TranscriptKind::Tool
            || self.tool_call_id.is_none()
            || self.has_attachments()
            || self.tool_presentation.is_some()
            || self.tool_review.is_some()
            || self.streaming
            || matches!(details.metadata.category, Some(ToolCategory::Change))
        {
            return self.clone();
        }
        let mut item = self.clone();
        let metadata = farcaster_agent_protocol::ToolMetadata {
            category: details.metadata.category,
            title: details.metadata.title.clone(),
            targets: details.metadata.targets.clone(),
            native: None,
        };
        item.tool_details = Some(Arc::new(ToolDetails {
            name: details.name.clone(),
            arguments: Value::Null,
            result: None,
            metadata,
            state: details.state,
            viewport: Some(ToolPreview {
                fingerprint: fingerprint(self),
                summary: details.summary(),
            }),
        }));
        item.text = String::new();
        item.tool_output = String::new();
        item
    }
}

/// A compact tool can match only its own complete source item, including its hidden output.
pub fn viewport_item_matches(preview: &TranscriptItem, item: &TranscriptItem) -> bool {
    if let Some(stamp) = preview
        .tool_details
        .as_ref()
        .and_then(|d| d.viewport.as_ref())
    {
        return item.kind == TranscriptKind::Tool
            && !item.has_attachments()
            && preview.tool_call_id == item.tool_call_id
            && item
                .tool_details
                .as_ref()
                .is_some_and(|d| d.viewport.is_none())
            && stamp.fingerprint == fingerprint(item);
    }
    preview == item
}
