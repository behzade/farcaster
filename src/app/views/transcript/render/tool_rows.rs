use std::sync::Arc;

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};

use crate::app::{
    FarcasterApp,
    ui::{
        assets::AppIcon,
        persistent_vec::PersistentVec,
        primitives::{AppIconSize, app_icon},
        theme::{MONO_FONT_FAMILY, THEME, UI_FONT_FAMILY},
    },
    views::transcript::{
        conversation::{ToolExecutionState, ToolReviewState, TranscriptItem, TranscriptKind},
        tool_changes,
    },
};

use super::{
    TRANSCRIPT_HORIZONTAL_PADDING, disclosure_detail, fenced_text, selectable_text, technical_text,
    toggle_transcript_item,
};

pub(super) fn render_activity_group(
    key: usize,
    items: &PersistentVec<Arc<TranscriptItem>>,
    start: usize,
    len: usize,
    expanded: bool,
    disclosure_states: &std::collections::HashMap<usize, bool>,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let group_items = || items.iter_range(start..start + len);
    let summary = activity_summary(group_items().map(AsRef::as_ref));
    // Attention states are standalone rows, never members of this group.
    let status = group_items()
        .filter(|item| item.kind == TranscriptKind::Tool)
        .all(|item| item_status(item) == Some(ToolStatus::Succeeded))
        .then_some(ToolStatus::Succeeded);
    let disclosure_label = format!(
        "{} activity details for {summary}. {}",
        if expanded { "Collapse" } else { "Expand" },
        status.map_or("No result", ToolStatus::label)
    );
    div()
        .id(("activity-group", key))
        .w_full()
        .px(TRANSCRIPT_HORIZONTAL_PADDING)
        .py(px(2.0))
        .flex()
        .flex_col()
        .child(
            tool_changes::title_row(
                ("activity-title", key),
                disclosure_label,
                toggle_transcript_item(entity.clone(), key, expanded),
            )
            .aria_expanded(expanded)
            .child(app_icon(
                if expanded {
                    AppIcon::CaretDown
                } else {
                    AppIcon::CaretRight
                },
                AppIconSize::Inline,
            ))
            .child(tool_changes::tool_label("Activity"))
            .child(
                technical_text(("activity-summary", key), summary)
                    .flex_1()
                    .min_w_0()
                    .font_family(UI_FONT_FAMILY)
                    .text_color(THEME.colors.muted),
            ),
        )
        .when(expanded, |group| {
            group.child(
                disclosure_detail()
                    .flex()
                    .flex_col()
                    .gap(THEME.space.xs)
                    .children(group_items().enumerate().map(|(offset, item)| {
                        let index = start + offset;
                        let child_expanded =
                            disclosure_states.get(&index).copied().unwrap_or(false);
                        if item.kind == TranscriptKind::Thinking {
                            super::render_thinking(index, item, child_expanded, entity.clone())
                        } else {
                            render_tool(index, item, child_expanded, entity.clone())
                        }
                    })),
            )
        })
        .into_any_element()
}

pub(super) fn render_tool(
    key: usize,
    item: &TranscriptItem,
    expanded: bool,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let status = item_status(item);
    let presentation = item.tool_presentation.as_ref().filter(|presentation| {
        !presentation.path().is_empty()
            && item
                .tool_details
                .as_ref()
                .is_none_or(|details| details.metadata.targets.len() <= 1)
    });
    let summary = item.tool_details.as_ref().map_or_else(
        || {
            if item.streaming && item.tool_call_id.is_none() {
                "Preparing tool call".to_owned()
            } else {
                item.label.clone()
            }
        },
        |details| details.summary(),
    );
    let disclosure_label = format!(
        "{} {summary} details. {}",
        if expanded { "Collapse" } else { "Expand" },
        status.map_or("No result", ToolStatus::label)
    );
    div()
        .id(("tool-row", key))
        .w_full()
        .px(TRANSCRIPT_HORIZONTAL_PADDING)
        .py(px(2.0))
        .flex()
        .flex_col()
        .child(
            tool_changes::title_row(
                ("tool-title", key),
                disclosure_label,
                toggle_transcript_item(entity.clone(), key, expanded),
            )
            .aria_expanded(expanded)
            .child(status_slot(status))
            .when_some(presentation, |row, presentation| {
                row.child(tool_changes::tool_label(item.label.clone()))
                    .child(tool_changes::file_summary(presentation))
            })
            .when(presentation.is_none(), |row| {
                row.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(THEME.type_scale.body_small)
                        .text_color(THEME.colors.muted)
                        .child(summary),
                )
            })
            .children(status.and_then(status_badge)),
        )
        .when(expanded, |tool| {
            tool.child(
                disclosure_detail()
                    .children(file_links(key, item, entity))
                    .child(expanded_tool_body(("tool-detail", key), item)),
            )
        })
        .into_any_element()
}

fn activity_summary<'a>(items: impl Iterator<Item = &'a TranscriptItem>) -> String {
    use crate::agents::ToolCategory;
    let mut counts = [0usize; 7];
    let mut calls = 0;
    let mut unknown = false;
    for item in items.filter(|item| item.kind == TranscriptKind::Tool) {
        calls += 1;
        let category = item
            .tool_details
            .as_ref()
            .and_then(|details| details.metadata.category);
        let slot = match category {
            Some(ToolCategory::Read) => 0,
            Some(ToolCategory::Search) => 1,
            Some(ToolCategory::List) => 2,
            Some(ToolCategory::Change) => 3,
            Some(ToolCategory::Execute) => 4,
            Some(ToolCategory::Fetch) => 5,
            Some(ToolCategory::Delegate) => 6,
            Some(ToolCategory::Other) | None => {
                unknown = true;
                continue;
            }
        };
        counts[slot] += 1;
    }
    if unknown || counts.iter().filter(|count| **count > 0).count() > 3 {
        return format!("{calls} {}", if calls == 1 { "call" } else { "calls" });
    }
    counts
        .into_iter()
        .zip([
            ("read", "reads"),
            ("search", "searches"),
            ("listing", "listings"),
            ("change", "changes"),
            ("command", "commands"),
            ("fetch", "fetches"),
            ("agent task", "agent tasks"),
        ])
        .filter(|(count, _)| *count > 0)
        .map(|(count, (singular, plural))| {
            format!("{count} {}", if count == 1 { singular } else { plural })
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn file_links(
    key: usize,
    item: &TranscriptItem,
    entity: WeakEntity<FarcasterApp>,
) -> Vec<AnyElement> {
    if !has_file_links(item) {
        return Vec::new();
    }
    let mut paths = item
        .tool_details
        .as_ref()
        .map(|details| details.metadata.targets.clone())
        .unwrap_or_default();
    if paths.is_empty()
        && let Some(presentation) = &item.tool_presentation
    {
        paths.push(presentation.path().to_owned());
    }
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .filter(|path| !path.is_empty() && seen.insert(path.clone()))
        .enumerate()
        .map(|(offset, path)| {
            let entity = entity.clone();
            let label = format!("Open {path} in Neovim");
            let line = item
                .tool_presentation
                .as_ref()
                .filter(|presentation| presentation.path() == path)
                .and_then(|presentation| presentation.first_changed_line());
            tool_changes::title_row(
                format!("tool-file-{key}-{offset}"),
                label.clone(),
                move |window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.open_file_editor_at_line(path.clone().into(), line, window, cx)
                    });
                },
            )
            .text_size(THEME.type_scale.body_small)
            .text_color(THEME.colors.accent)
            .child(label)
            .into_any_element()
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolStatus {
    Reviewing,
    Rejected,
    Running,
    Succeeded,
    Failed,
}

impl ToolStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Reviewing => "Awaiting approval",
            Self::Rejected => "Rejected",
            Self::Running => "Running",
            Self::Succeeded => "Succeeded",
            Self::Failed => "Failed",
        }
    }
    fn color(self) -> gpui::Rgba {
        match self {
            Self::Reviewing | Self::Rejected => THEME.colors.warning,
            Self::Failed => THEME.colors.error,
            Self::Running | Self::Succeeded => THEME.colors.muted,
        }
    }
    fn icon(self) -> AppIcon {
        match self {
            Self::Reviewing => AppIcon::Shield,
            Self::Rejected | Self::Failed => AppIcon::XCircle,
            Self::Running => AppIcon::SpinnerGap,
            Self::Succeeded => AppIcon::CheckCircle,
        }
    }
}

fn item_status(item: &TranscriptItem) -> Option<ToolStatus> {
    match item.tool_review.as_ref().map(|review| review.state) {
        Some(ToolReviewState::Reviewing) => return Some(ToolStatus::Reviewing),
        Some(ToolReviewState::Blocked) => return Some(ToolStatus::Rejected),
        _ => {}
    }
    if item.is_error {
        return Some(ToolStatus::Failed);
    }
    if item.streaming {
        return Some(ToolStatus::Running);
    }
    if let Some(details) = &item.tool_details {
        return match details.state {
            ToolExecutionState::Pending => None,
            ToolExecutionState::Running => Some(ToolStatus::Running),
            ToolExecutionState::Succeeded => Some(ToolStatus::Succeeded),
            ToolExecutionState::Failed => Some(ToolStatus::Failed),
        };
    }
    (!item.tool_output.is_empty()).then_some(ToolStatus::Succeeded)
}

fn has_file_links(item: &TranscriptItem) -> bool {
    use crate::agents::ToolCategory;
    item_status(item) == Some(ToolStatus::Succeeded)
        && (item
            .tool_presentation
            .as_ref()
            .is_some_and(|presentation| !presentation.path().is_empty())
            || item.tool_details.as_ref().is_some_and(|details| {
                matches!(
                    details.metadata.category,
                    Some(ToolCategory::Read | ToolCategory::Change)
                ) && details
                    .metadata
                    .targets
                    .iter()
                    .any(|target| !target.is_empty())
            }))
}

fn status_slot(status: Option<ToolStatus>) -> AnyElement {
    div()
        .w(THEME.icons.control)
        .h(THEME.icons.control)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .when_some(status, |slot, status| {
            slot.text_color(status.color())
                .child(app_icon(status.icon(), AppIconSize::Inline))
        })
        .into_any_element()
}

fn status_badge(status: ToolStatus) -> Option<AnyElement> {
    (status != ToolStatus::Succeeded).then(|| {
        div()
            .flex_none()
            .text_size(THEME.type_scale.caption)
            .text_color(status.color())
            .child(status.label())
            .into_any_element()
    })
}

fn expanded_tool_body(id: impl Into<gpui::ElementId>, item: &TranscriptItem) -> AnyElement {
    let mut detail = item
        .tool_details
        .as_ref()
        .map_or_else(|| item.text.clone(), |details| details.inspection_text());
    if item.tool_details.is_none() && !item.tool_output.is_empty() {
        if !detail.is_empty() {
            detail.push_str("\n\n");
        }
        detail.push_str(&item.tool_output);
    }
    if let Some(review) = &item.tool_review {
        if !detail.is_empty() {
            detail.push_str("\n\n");
        }
        detail.push_str("Approval review: ");
        detail.push_str(review.state.label());
        if let Some(review_detail) = &review.detail {
            detail.push('\n');
            detail.push_str(review_detail);
        }
    }
    selectable_text(id, fenced_text(&detail))
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small)
        .text_color(if item.is_error {
            THEME.colors.error
        } else {
            THEME.colors.muted
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::views::transcript::conversation::{ConversationState, ToolReview};
    use serde_json::json;

    fn write_item() -> TranscriptItem {
        let mut state = ConversationState::default();
        state.reduce(&json!({
            "type": "tool_execution_start", "toolCallId": "write-1", "toolName": "write",
            "args": {"path": "src/main.rs", "content": "fn main() {}\n"}
        }));
        state.reduce(&json!({
            "type": "tool_execution_end", "toolCallId": "write-1", "isError": false,
            "result": {"content": [{"type": "text", "text": "Wrote src/main.rs"}]}
        }));
        (*state.items[0]).clone()
    }

    #[test]
    fn only_successful_file_changes_open_editor() {
        let mut item = write_item();
        assert!(has_file_links(&item));
        item.streaming = true;
        assert!(!has_file_links(&item));
        item.streaming = false;
        item.is_error = true;
        assert!(!has_file_links(&item));
        item.is_error = false;
        item.tool_presentation = None;
        assert!(has_file_links(&item)); // Native targets do not require a diff preview.
        Arc::make_mut(item.tool_details.as_mut().unwrap())
            .metadata
            .targets
            .clear();
        assert!(!has_file_links(&item));
    }

    #[test]
    fn approval_replaces_execution_status_instead_of_coexisting() {
        let mut item = write_item();
        item.streaming = true;
        item.tool_review = Some(ToolReview {
            state: ToolReviewState::Reviewing,
            detail: None,
        });
        assert_eq!(item_status(&item), Some(ToolStatus::Reviewing));
        assert!(!has_file_links(&item));
        item.is_error = true;
        item.tool_review.as_mut().unwrap().state = ToolReviewState::Blocked;
        assert_eq!(item_status(&item), Some(ToolStatus::Rejected));
        assert!(!has_file_links(&item));
        item.tool_review.as_mut().unwrap().state = ToolReviewState::Approved;
        assert_eq!(item_status(&item), Some(ToolStatus::Failed));
        item.is_error = false;
        assert_eq!(item_status(&item), Some(ToolStatus::Running));
        item.streaming = false;
        assert_eq!(item_status(&item), Some(ToolStatus::Succeeded));
        assert!(has_file_links(&item));
    }

    #[test]
    fn only_success_is_quiet_beside_the_status_icon() {
        assert!(status_badge(ToolStatus::Succeeded).is_none());
        for status in [
            ToolStatus::Reviewing,
            ToolStatus::Rejected,
            ToolStatus::Running,
            ToolStatus::Failed,
        ] {
            assert!(status_badge(status).is_some());
        }
    }

    #[test]
    fn activity_summary_counts_calls_not_events_or_claimed_files() {
        let mut state = ConversationState::default();
        for (id, name, metadata) in [
            (
                "a",
                "read",
                json!({"category":"read", "targets":["same.rs"]}),
            ),
            (
                "b",
                "read",
                json!({"category":"read", "targets":["same.rs"]}),
            ),
            ("c", "bash", json!({"category":"execute"})),
        ] {
            state.reduce(&json!({"type":"tool_execution_start", "toolCallId":id, "toolName":name, "args":{}, "toolMetadata":metadata}));
            state.reduce(&json!({"type":"tool_execution_update", "toolCallId":id, "partialResult":{"content":[]}}));
            state.reduce(&json!({"type":"tool_execution_end", "toolCallId":id, "result":{"content":[]}, "isError":false}));
        }
        assert_eq!(
            activity_summary(state.items.iter().map(AsRef::as_ref)),
            "2 reads · 1 command"
        );
        assert_eq!(item_status(&state.items[2]), Some(ToolStatus::Succeeded));
        state.reduce(&json!({"type":"tool_execution_start", "toolCallId":"custom", "toolName":"mcp_database", "args":{"path":"not-a-file"}}));
        assert_eq!(
            activity_summary(state.items.iter().map(AsRef::as_ref)),
            "4 calls"
        );
    }

    #[test]
    fn native_file_targets_are_openable_without_a_diff_preview() {
        let mut state = ConversationState::default();
        state.reduce(&json!({"type":"tool_execution_start", "toolCallId":"acp", "toolName":"Inspect", "args":{}, "toolMetadata":{"category":"read", "targets":["src/main.rs"]}}));
        assert!(!has_file_links(&state.items[0]));
        state.reduce(&json!({"type":"tool_execution_end", "toolCallId":"acp", "result":{"content":[]}, "isError":false}));
        assert!(state.items[0].tool_presentation.is_none());
        assert!(has_file_links(&state.items[0]));
    }
}
