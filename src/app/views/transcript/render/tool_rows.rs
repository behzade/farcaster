use std::sync::Arc;

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _, Styled as _,
    WeakEntity, div, prelude::FluentBuilder as _, px,
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
    toggle_transcript_item, tool_state, tool_target, transcript_title_row,
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
    let failed = group_items().filter(|item| item.is_error).count();
    let running = group_items().filter(|item| item.streaming).count();
    let target = (len == 1).then(|| tool_target(&items[start].text));
    let has_target = target.as_ref().is_some_and(|target| !target.is_empty());
    let summary = if len == 1 {
        "Read".to_owned()
    } else {
        format!("Read {len} files")
    };
    let completed = group_items()
        .all(|item| !item.streaming && (item.is_error || !item.tool_output.is_empty()));
    let state = tool_state(running > 0, failed, completed);
    let disclosure_label = format!(
        "read call details for {summary}. {}",
        state.map_or("No result", |state| state.label)
    );
    div()
        .id(("read-group", key))
        .w_full()
        .px(TRANSCRIPT_HORIZONTAL_PADDING)
        .py(px(2.0))
        .flex()
        .flex_col()
        .child(
            transcript_title_row(
                ("read-title", key),
                expanded,
                true,
                disclosure_label,
                key,
                entity.clone(),
            )
            .child(
                div()
                    .text_size(THEME.type_scale.body_small)
                    .text_color(THEME.colors.muted)
                    .when(!has_target, |label| label.flex_1())
                    .child(summary),
            )
            .children(target.filter(|target| !target.is_empty()).map(|target| {
                technical_text(("read-target", key), target)
                    .flex_1()
                    .min_w_0()
                    .text_color(THEME.colors.text)
            }))
            .children(state.map(|state| {
                div()
                    .flex_none()
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.subtle)
                    .child(state.glyph)
            })),
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
    let state = tool_state(
        item.streaming,
        usize::from(item.is_error),
        !item.streaming && (item.is_error || !item.tool_output.is_empty()),
    );
    let presentation = item.tool_presentation.as_ref();
    let target =
        presentation.map_or_else(|| tool_target(&item.text), |change| change.path().into());
    let has_target = !target.is_empty();
    let detail_label = if has_target {
        format!("{} tool call details for {target}", item.label)
    } else {
        format!("{} tool call details", item.label)
    };
    let review_label = item
        .tool_review
        .as_ref()
        .map(|review| format!("Approval review {}", review.state.label()));
    let disclosure_label = format!(
        "{detail_label}. {}{}",
        state.map_or("No result", |state| state.label),
        review_label.map_or_else(String::new, |label| format!(". {label}")),
    );
    if let Some(presentation) = presentation {
        let source = presentation.clone();
        let open_entity = entity.clone();
        return tool_changes::render(
            &item.label,
            presentation,
            key,
            state.map(|state| state.glyph),
            tool_review_indicator(item),
            expanded,
            disclosure_label,
            toggle_transcript_item(entity.clone(), key, expanded),
            expanded.then(|| expanded_tool_body(("tool-detail", key), item)),
            move |window, cx| {
                let _ = open_entity.update(cx, |this, cx| {
                    this.open_file_editor_at_line(
                        source.path().into(),
                        source.first_changed_line(),
                        window,
                        cx,
                    );
                });
            },
        );
    }
    div()
        .id(("tool-row", key))
        .w_full()
        .px(TRANSCRIPT_HORIZONTAL_PADDING)
        .py(px(2.0))
        .flex()
        .flex_col()
        .child(
            transcript_title_row(
                ("tool-title", key),
                expanded,
                true,
                disclosure_label,
                key,
                entity,
            )
            .child(
                div()
                    .text_size(THEME.type_scale.body_small)
                    .text_color(THEME.colors.muted)
                    .when(!has_target, |label| label.flex_1())
                    .child(item.label.clone()),
            )
            .when(has_target, |row| {
                row.child(
                    technical_text(("tool-target", key), target)
                        .flex_1()
                        .min_w_0()
                        .text_color(THEME.colors.text),
                )
            })
            .children(tool_review_indicator(item))
            .children(state.map(|state| {
                div()
                    .flex_none()
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.subtle)
                    .child(state.glyph)
            })),
        )
        .when(expanded, |tool| {
            tool.child(disclosure_detail().child(expanded_tool_body(("tool-detail", key), item)))
        })
        .into_any_element()
}

fn tool_review_indicator(item: &TranscriptItem) -> Option<AnyElement> {
    let review = item.tool_review.as_ref()?;
    let (icon, color) = match review.state {
        ToolReviewState::Reviewing => (AppIcon::Shield, THEME.colors.warning),
        ToolReviewState::Approved => (AppIcon::CheckCircle, THEME.colors.success),
        ToolReviewState::Blocked => (AppIcon::XCircle, THEME.colors.error),
    };
    Some(
        div()
            .flex_none()
            .text_color(color)
            .child(app_icon(icon, AppIconSize::Inline))
            .into_any_element(),
    )
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
