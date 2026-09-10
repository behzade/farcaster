use std::{path::Path, sync::Arc};

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
    TRANSCRIPT_HORIZONTAL_PADDING, disclosure_detail, fenced_text, selectable_text,
    toggle_transcript_item,
};

#[path = "tool_rows/changed_files.rs"]
mod changed_files;

#[path = "tool_preview.rs"]
mod tool_preview;
use tool_preview::ToolPreview;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_activity_group(
    font_scale: f32,
    key: usize,
    items: &PersistentVec<Arc<TranscriptItem>>,
    start: usize,
    len: usize,
    expanded: bool,
    disclosure_states: &std::collections::HashMap<usize, bool>,
    file_tree: Option<&crate::app::ui::change_tree::ChangeTreeState>,
    entity: WeakEntity<FarcasterApp>,
    cx: &gpui::App,
) -> AnyElement {
    let group_items = || items.iter_range(start..start + len);
    let summary = activity_summary(group_items().map(AsRef::as_ref));
    let disclosure_label = format!(
        "{} activity details for {summary}",
        if expanded { "Collapse" } else { "Expand" },
    );
    let files = changed_files::render(key, items, start, len, file_tree, entity.clone(), cx);
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
            .font_family(UI_FONT_FAMILY)
            .text_size(THEME.type_scale.body_small * font_scale)
            .line_height(THEME.type_scale.line_body * font_scale)
            .text_color(THEME.colors.muted)
            .child(summary),
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
                            super::render_thinking(
                                font_scale,
                                index,
                                item,
                                child_expanded,
                                entity.clone(),
                            )
                        } else {
                            render_tool(font_scale, index, item, child_expanded, entity.clone(), cx)
                        }
                    })),
            )
        })
        .child(files)
        .into_any_element()
}

pub(super) fn render_tool(
    font_scale: f32,
    key: usize,
    item: &TranscriptItem,
    expanded: bool,
    entity: WeakEntity<FarcasterApp>,
    cx: &gpui::App,
) -> AnyElement {
    let status = item_status(item);
    let project = entity
        .upgrade()
        .map(|entity| entity.read(cx).workspace_project());
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let presentation = item.tool_presentation.as_ref().filter(|presentation| {
        !presentation.path().is_empty()
            && item
                .tool_details
                .as_ref()
                .is_none_or(|details| details.metadata.targets.len() <= 1)
    });
    let summary = tool_summary(item);
    let read_target = direct_read_target(item).map(str::to_owned);
    let opens_file = read_target.is_some();
    let expanded = expanded && !opens_file;
    let title_entity = entity.clone();
    let toggle = toggle_transcript_item(entity.clone(), key, expanded);
    let title_label = match &read_target {
        Some(path) => format!("Open current file: {path}"),
        None => format!(
            "{} {summary} details. {}",
            if expanded { "Collapse" } else { "Expand" },
            status.map_or("No result", ToolStatus::label)
        ),
    };
    div()
        .id(("tool-row", key))
        .w_full()
        .px(TRANSCRIPT_HORIZONTAL_PADDING)
        .py(px(2.0))
        .flex()
        .flex_col()
        .child(
            tool_changes::title_row(("tool-title", key), title_label, move |window, cx| {
                if let Some(path) = &read_target {
                    let _ = title_entity.update(cx, |this, cx| {
                        this.open_file_editor_at_line(path.clone().into(), None, window, cx)
                    });
                } else {
                    toggle(window, cx);
                }
            })
            .when(!opens_file, |row| row.aria_expanded(expanded))
            .when(
                status.is_some_and(|status| status != ToolStatus::Succeeded),
                |row| row.child(status_slot(status)),
            )
            .when_some(presentation, |row, presentation| {
                let label = tool_changes::file_label(
                    presentation.path(),
                    project.as_deref(),
                    home.as_deref(),
                );
                if status == Some(ToolStatus::Succeeded) {
                    row.child(tool_changes::file_summary(presentation, label))
                } else {
                    row.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(MONO_FONT_FAMILY)
                            .text_size(THEME.type_scale.body_small * font_scale)
                            .child(label),
                    )
                }
            })
            .when(presentation.is_none(), |row| {
                row.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(THEME.type_scale.body_small * font_scale)
                        .text_color(THEME.colors.muted)
                        .child(summary),
                )
            }),
        )
        .when(expanded, |tool| {
            tool.child(
                disclosure_detail()
                    .id(("tool-detail-scroll", key))
                    .max_h(THEME.layout.tool_max_height)
                    .overflow_y_scroll()
                    .children(file_links(
                        font_scale,
                        key,
                        item,
                        entity,
                        project.as_deref(),
                    ))
                    .child(expanded_tool_body(font_scale, ("tool-detail", key), item)),
            )
        })
        .into_any_element()
}

pub(super) fn tool_summary(item: &TranscriptItem) -> String {
    item.tool_details.as_ref().map_or_else(
        || {
            if item.streaming && item.tool_call_id.is_none() {
                "Preparing tool call".to_owned()
            } else {
                item.label.clone()
            }
        },
        |details| details.summary(),
    )
}

pub(super) fn activity_summary<'a>(items: impl Iterator<Item = &'a TranscriptItem>) -> String {
    use crate::agents::ToolCategory;
    let mut counts = [0usize; 8];
    let mut calls = 0;
    let mut custom_title = None;
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
                custom_title = Some(
                    item.tool_details
                        .as_ref()
                        .map_or_else(|| item.label.clone(), |details| details.summary()),
                );
                7
            }
        };
        counts[slot] += 1;
    }
    if calls == 1
        && let Some(title) = custom_title
    {
        return title;
    }
    let mut categories = counts
        .into_iter()
        .zip([
            ("read", "reads"),
            ("search", "searches"),
            ("listing", "listings"),
            ("edit", "edits"),
            ("command", "commands"),
            ("fetch", "fetches"),
            ("agent task", "agent tasks"),
            ("other action", "other actions"),
        ])
        .filter(|(count, _)| *count > 0)
        .collect::<Vec<_>>();
    if categories.len() > 3 {
        let remaining = categories.drain(2..).map(|(count, _)| count).sum();
        categories.push((remaining, ("other action", "other actions")));
    }
    categories
        .into_iter()
        .map(|(count, (singular, plural))| {
            format!("{count} {}", if count == 1 { singular } else { plural })
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn file_links(
    font_scale: f32,
    key: usize,
    item: &TranscriptItem,
    entity: WeakEntity<FarcasterApp>,
    project: Option<&Path>,
) -> Vec<AnyElement> {
    file_targets(item)
        .enumerate()
        .map(|(offset, path)| {
            let path = path.to_owned();
            let entity = entity.clone();
            let label = format!("Open current file: {path}");
            let line = file_target_line(item, &path, project);
            let diff_enabled = item.tool_details.as_ref().is_some_and(|details| {
                details.metadata.category == Some(crate::agents::ToolCategory::Change)
            });
            tool_changes::file_row(
                format!("tool-file-{key}-{offset}"),
                label.clone(),
                diff_enabled,
                move |diff, window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.open_file_editor_with_diff(path.clone().into(), line, diff, window, cx)
                    });
                },
            )
            .text_size(THEME.type_scale.body_small * font_scale)
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
    match item.tool_execution_state()? {
        ToolExecutionState::Pending => None,
        ToolExecutionState::Running => Some(ToolStatus::Running),
        ToolExecutionState::Succeeded => Some(ToolStatus::Succeeded),
        ToolExecutionState::Failed => Some(ToolStatus::Failed),
    }
}

fn file_target_line(item: &TranscriptItem, path: &str, project: Option<&Path>) -> Option<u64> {
    item.tool_presentation
        .as_ref()
        .filter(|presentation| {
            tool_changes::file_path(presentation.path(), project)
                == tool_changes::file_path(path, project)
        })
        .and_then(|presentation| presentation.first_changed_line())
        .or_else(|| {
            changed_files::recorded_edits(item, path, project)?
                .iter()
                .find_map(|edit| {
                    if edit.old == edit.new {
                        return None;
                    }
                    let context = edit
                        .old
                        .iter()
                        .zip(&edit.new)
                        .take_while(|(old, new)| old == new)
                        .count();
                    u64::try_from(edit.start.checked_add(context)?.checked_add(1)?).ok()
                })
        })
}

fn direct_read_target(item: &TranscriptItem) -> Option<&str> {
    if item.tool_details.as_ref()?.metadata.category != Some(crate::agents::ToolCategory::Read) {
        return None;
    }
    let mut targets = file_targets(item);
    let path = targets.next()?;
    targets.next().is_none().then_some(path)
}

fn file_targets(item: &TranscriptItem) -> impl Iterator<Item = &str> {
    use crate::agents::ToolCategory;
    let metadata = item.tool_details.as_ref().map(|details| &details.metadata);
    let targets = metadata.map_or(&[][..], |metadata| metadata.targets.as_slice());
    let path = item
        .tool_presentation
        .as_ref()
        .map(|presentation| presentation.path())
        .filter(|path| !path.is_empty());
    let enabled = item_status(item) == Some(ToolStatus::Succeeded)
        && (path.is_some()
            || matches!(
                metadata.and_then(|metadata| metadata.category),
                Some(ToolCategory::Read | ToolCategory::Change)
            ));
    let mut seen = std::collections::HashSet::new();
    targets
        .iter()
        .map(String::as_str)
        .chain(path.filter(|_| targets.is_empty()))
        .filter(move |path| enabled && !path.is_empty() && seen.insert(*path))
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

fn expanded_tool_body(
    font_scale: f32,
    id: impl Into<gpui::ElementId>,
    item: &TranscriptItem,
) -> AnyElement {
    selectable_text(font_scale, id, fenced_text(&tool_body_text(item)))
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small * font_scale)
        .text_color(if item.is_error {
            THEME.colors.error
        } else {
            THEME.colors.muted
        })
        .into_any_element()
}

fn tool_body_text(item: &TranscriptItem) -> String {
    let mut detail = ToolPreview::default();
    detail.push_str(&item.text);
    if let Some(command) = item
        .tool_details
        .as_ref()
        .and_then(|details| details.command_preview())
        && !detail.contains(command)
    {
        detail.push_str("\n\nCommand:\n");
        detail.push_str(command);
    }
    if !item.tool_output.is_empty() {
        if !detail.is_empty() {
            detail.push_str("\n\nOutput:\n");
        }
        detail.push_str(&item.tool_output);
    }
    if detail.is_empty() {
        if let Some(details) = &item.tool_details {
            let _ = details.write_inspection(&mut detail);
        } else {
            detail.push_str("No details available");
        }
    }
    if let Some(review) = &item.tool_review {
        if !detail.is_empty() {
            detail.push_str("\n\n");
        }
        detail.push_str("Approval review: ");
        detail.push_str(review.state.label());
        if let Some(review_detail) = &review.detail {
            detail.push_str("\n");
            detail.push_str(review_detail);
        }
    }
    detail.finish()
}

#[cfg(test)]
#[path = "tool_rows_tests.rs"]
mod tests;
