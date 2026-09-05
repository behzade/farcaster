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
        theme::{MONO_FONT_FAMILY, THEME},
    },
    views::transcript::{
        conversation::{ToolReviewState, TranscriptItem},
        tool_changes,
    },
};

use super::{
    TRANSCRIPT_HORIZONTAL_PADDING, disclosure_detail, fenced_text, selectable_text, technical_text,
    toggle_transcript_item, tool_state, tool_target,
};

pub(super) fn render_read_group(
    key: usize,
    items: &PersistentVec<Arc<TranscriptItem>>,
    start: usize,
    len: usize,
    expanded: bool,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let group_items = || items.iter_range(start..start + len);
    let target = (len == 1).then(|| tool_target(&items[start].text));
    let summary = if len == 1 {
        "Read".to_owned()
    } else {
        format!("{len} files")
    };
    let status = group_status(group_items().map(|item| item_status(item)));
    let disclosure_label = format!(
        "{} read call details for {summary}. {}",
        if expanded { "Collapse" } else { "Expand" },
        status.map_or("No result", ToolStatus::label)
    );
    div()
        .id(("read-group", key))
        .w_full()
        .px(TRANSCRIPT_HORIZONTAL_PADDING)
        .py(px(2.0))
        .flex()
        .flex_col()
        .child(
            tool_changes::title_row(
                ("read-title", key),
                disclosure_label,
                toggle_transcript_item(entity.clone(), key, expanded),
            )
            .aria_expanded(expanded)
            .child(status_slot(status))
            .child(tool_changes::tool_label("Read"))
            .child(
                technical_text(
                    ("read-target", key),
                    target
                        .filter(|target| !target.is_empty())
                        .unwrap_or(summary),
                )
                .flex_1()
                .min_w_0()
                .text_color(THEME.colors.text),
            )
            .children(status.and_then(status_badge)),
        )
        .when(expanded, |group| {
            group.child(
                disclosure_detail()
                    .flex()
                    .flex_col()
                    .gap(THEME.space.xs)
                    .children(group_items().enumerate().map(|(index, item)| {
                        expanded_tool_body(format!("read-detail-{key}-{index}"), item)
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
    let presentation = item.tool_presentation.as_ref();
    let target =
        presentation.map_or_else(|| tool_target(&item.text), |change| change.path().into());
    let has_target = !target.is_empty();
    let detail_label = if has_target {
        format!("{} tool call details for {target}", item.label)
    } else {
        format!("{} tool call details", item.label)
    };
    let opens_file = opens_file(item);
    let disclosure_label = if opens_file {
        format!("Open {target} in Neovim")
    } else {
        format!(
            "{} {detail_label}. {}",
            if expanded { "Collapse" } else { "Expand" },
            status.map_or("No result", ToolStatus::label)
        )
    };
    let source = presentation.cloned().filter(|_| opens_file);
    let click_entity = entity.clone();
    div()
        .id(("tool-row", key))
        .w_full()
        .px(TRANSCRIPT_HORIZONTAL_PADDING)
        .py(px(2.0))
        .flex()
        .flex_col()
        .child(
            tool_changes::title_row(("tool-title", key), disclosure_label, move |window, cx| {
                let _ = click_entity.update(cx, |this, cx| {
                    if let Some(source) = &source {
                        this.open_file_editor_at_line(
                            source.path().into(),
                            source.first_changed_line(),
                            window,
                            cx,
                        );
                    } else {
                        this.set_transcript_item_expanded(key, !expanded, cx);
                    }
                });
            })
            .when(!opens_file, |row| row.aria_expanded(expanded))
            .child(status_slot(status))
            .child(tool_changes::tool_label(item.label.clone()))
            .when_some(presentation, |row, presentation| {
                row.child(tool_changes::file_summary(presentation))
            })
            .when(presentation.is_none() && has_target, |row| {
                row.child(
                    technical_text(("tool-target", key), target)
                        .flex_1()
                        .min_w_0()
                        .text_color(THEME.colors.text),
                )
            })
            .children(status.and_then(status_badge)),
        )
        .when(expanded, |tool| {
            tool.child(disclosure_detail().child(expanded_tool_body(("tool-detail", key), item)))
        })
        .into_any_element()
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

fn status_from_label(label: &str) -> ToolStatus {
    match label {
        "Failed" => ToolStatus::Failed,
        "Working" => ToolStatus::Running,
        _ => ToolStatus::Succeeded,
    }
}

fn item_status(item: &TranscriptItem) -> Option<ToolStatus> {
    match item.tool_review.as_ref().map(|review| review.state) {
        Some(ToolReviewState::Reviewing) => return Some(ToolStatus::Reviewing),
        Some(ToolReviewState::Blocked) => return Some(ToolStatus::Rejected),
        _ => {}
    }
    tool_state(
        item.streaming,
        usize::from(item.is_error),
        !item.streaming && (item.is_error || !item.tool_output.is_empty()),
    )
    .map(|state| status_from_label(state.label))
}

fn group_status(statuses: impl Iterator<Item = Option<ToolStatus>>) -> Option<ToolStatus> {
    let statuses = statuses.collect::<Vec<_>>();
    [
        ToolStatus::Reviewing,
        ToolStatus::Failed,
        ToolStatus::Rejected,
        ToolStatus::Running,
    ]
    .into_iter()
    .find(|status| statuses.contains(&Some(*status)))
    .or_else(|| {
        (!statuses.is_empty()
            && statuses
                .iter()
                .all(|status| *status == Some(ToolStatus::Succeeded)))
        .then_some(ToolStatus::Succeeded)
    })
}

pub(super) fn opens_file(item: &TranscriptItem) -> bool {
    item.tool_presentation.is_some() && item_status(item) == Some(ToolStatus::Succeeded)
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
    matches!(status, ToolStatus::Reviewing | ToolStatus::Rejected).then(|| {
        div()
            .flex_none()
            .text_size(THEME.type_scale.caption)
            .text_color(status.color())
            .child(status.label())
            .into_any_element()
    })
}

fn expanded_tool_body(id: impl Into<gpui::ElementId>, item: &TranscriptItem) -> AnyElement {
    let mut detail = String::new();
    if !item.text.is_empty() {
        detail.push_str(&item.text);
    }
    if !item.tool_output.is_empty() {
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
        assert!(opens_file(&item));
        item.streaming = true;
        assert!(!opens_file(&item));
        item.streaming = false;
        item.is_error = true;
        assert!(!opens_file(&item));
        item.is_error = false;
        item.tool_presentation = None;
        assert!(!opens_file(&item));
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
        assert!(!opens_file(&item));
        item.is_error = true;
        item.tool_review.as_mut().unwrap().state = ToolReviewState::Blocked;
        assert_eq!(item_status(&item), Some(ToolStatus::Rejected));
        assert!(!opens_file(&item));
        item.tool_review.as_mut().unwrap().state = ToolReviewState::Approved;
        assert_eq!(item_status(&item), Some(ToolStatus::Failed));
        item.is_error = false;
        assert_eq!(item_status(&item), Some(ToolStatus::Running));
        item.streaming = false;
        assert_eq!(item_status(&item), Some(ToolStatus::Succeeded));
        assert!(opens_file(&item));
    }

    #[test]
    fn rejection_and_failure_have_distinct_labels_and_colors() {
        assert_eq!(ToolStatus::Rejected.label(), "Rejected");
        assert_eq!(ToolStatus::Rejected.color(), THEME.colors.warning);
        assert_eq!(ToolStatus::Failed.label(), "Failed");
        assert_eq!(ToolStatus::Failed.color(), THEME.colors.error);
    }

    #[test]
    fn only_approval_states_need_text_beside_the_status_icon() {
        for status in [
            ToolStatus::Running,
            ToolStatus::Succeeded,
            ToolStatus::Failed,
        ] {
            assert!(status_badge(status).is_none());
        }
        for status in [ToolStatus::Reviewing, ToolStatus::Rejected] {
            assert!(status_badge(status).is_some());
        }
    }

    #[test]
    fn grouped_reads_preserve_approval_and_failure_states() {
        use ToolStatus::*;
        assert_eq!(
            group_status([Some(Succeeded), Some(Reviewing)].into_iter()),
            Some(Reviewing)
        );
        assert_eq!(
            group_status([Some(Succeeded), Some(Rejected)].into_iter()),
            Some(Rejected)
        );
        assert_eq!(
            group_status([Some(Failed), Some(Succeeded)].into_iter()),
            Some(Failed)
        );
        assert_eq!(group_status([Some(Succeeded), None].into_iter()), None);
        assert_eq!(group_status([Some(Succeeded)].into_iter()), Some(Succeeded));
    }
}
