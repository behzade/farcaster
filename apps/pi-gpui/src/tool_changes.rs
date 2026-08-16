//! Native, selectable presentations for file mutation tools.

use std::sync::Arc;

use gpui::{
    AnyElement, CursorStyle, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    Overflow, ParentElement as _, Role, StatefulInteractiveElement as _, StyleRefinement,
    Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px, rems, transparent_black,
};
use gpui_component::{
    FocusTrapElement as _,
    highlighter::HighlightTheme,
    text::{TextView, TextViewStyle},
};

use crate::{
    app::{OVERLAY_KEY_CONTEXT, PiApp},
    conversation::{EditDiffFormat, ToolPresentation, TranscriptItem},
    primitives::{ButtonTone, button, dialog_backdrop, dialog_surface},
    theme::{READING_FONT_FAMILY, THEME},
};

const INLINE_PREVIEW_LINES: usize = 4;
const MAX_MODAL_LINES: usize = 600;
const SOFT_WRAP_COLUMNS: usize = 12;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChangeModal {
    pub(crate) presentation: ToolPresentation,
    pub(crate) tool_call_id: Option<String>,
    pub(crate) key: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChangeKind {
    Context,
    Addition,
    Deletion,
    Ellipsis,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ChangeLine {
    kind: ChangeKind,
    number: Option<u64>,
    content: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SideLine {
    kind: ChangeKind,
    number: Option<u64>,
    content: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PairedLine {
    old: Option<SideLine>,
    new: Option<SideLine>,
}

pub(crate) fn render(
    presentation: &ToolPresentation,
    tool_call_id: Option<&str>,
    key: usize,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let (path, rows) = presentation_rows(presentation);
    let title = change_title(path);
    let (displayed, remaining) = preview_rows(&rows);
    let keyboard_presentation = presentation.clone();
    let click_presentation = presentation.clone();
    let keyboard_tool_call_id = tool_call_id.map(str::to_owned);
    let click_tool_call_id = keyboard_tool_call_id.clone();
    let keyboard_entity = entity.clone();
    div()
        .id(("tool-change", key))
        .role(Role::Button)
        .aria_label(format!("Open file change preview for {title}"))
        .tab_index(0)
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .rounded(THEME.radius)
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.canvas)
        .hover(|preview| preview.border_color(THEME.colors.accent))
        .focus(|preview| preview.border_color(THEME.colors.accent))
        .cursor(CursorStyle::PointingHand)
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                window.prevent_default();
                let presentation = keyboard_presentation.clone();
                let _ = keyboard_entity.update(cx, |this, cx| {
                    this.open_change_modal(
                        presentation,
                        keyboard_tool_call_id.clone(),
                        key,
                        window,
                        cx,
                    );
                });
            }
        })
        .on_click(move |_, window, cx| {
            let presentation = click_presentation.clone();
            let _ = entity.update(cx, |this, cx| {
                this.open_change_modal(presentation, click_tool_call_id.clone(), key, window, cx);
            });
        })
        .child(change_header(title, false, None))
        .children(
            displayed
                .iter()
                .enumerate()
                .map(|(index, row)| render_preview_line(row, key, index)),
        )
        .when(remaining > 0, |preview| {
            preview.child(
                div()
                    .px(THEME.space.sm)
                    .py(THEME.space.xs)
                    .border_t(THEME.border)
                    .border_color(THEME.colors.border)
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.muted)
                    .child(format!("+{remaining} more lines · Open full diff")),
            )
        })
        .into_any_element()
}

pub(crate) fn modal_presentation<'a>(
    modal: &'a ChangeModal,
    items: &'a [Arc<TranscriptItem>],
) -> &'a ToolPresentation {
    modal
        .tool_call_id
        .as_deref()
        .and_then(|tool_call_id| {
            items.iter().rev().find_map(|item| {
                (item.tool_call_id.as_deref() == Some(tool_call_id))
                    .then_some(item.tool_presentation.as_ref())
                    .flatten()
            })
        })
        .unwrap_or(&modal.presentation)
}

pub(crate) fn render_modal(
    modal: &ChangeModal,
    presentation: &ToolPresentation,
    focus: &FocusHandle,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let (path, rows) = presentation_rows(presentation);
    let title = change_title(path);
    let key = modal.key;
    let close_backdrop = entity.clone();
    let close_button = entity;
    let body = match presentation {
        ToolPresentation::Edit { .. } => render_edit_modal(&rows, key),
        ToolPresentation::Write { .. } => render_write_modal(&rows, key),
    };
    dialog_backdrop(("change-modal-backdrop", key), move |window, cx| {
        let _ = close_backdrop.update(cx, |this, cx| this.close_change_modal(window, cx));
    })
    .child(
        dialog_surface(("change-modal", key), format!("File change: {title}"))
            .track_focus(focus)
            .key_context(OVERLAY_KEY_CONTEXT)
            .w(px(1_040.0))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(change_header(
                title,
                true,
                Some(
                    button(
                        ("close-change-modal", key),
                        "Close",
                        ButtonTone::Quiet,
                        true,
                        move |window, cx| {
                            let _ = close_button
                                .update(cx, |this, cx| this.close_change_modal(window, cx));
                        },
                    )
                    .into_any_element(),
                ),
            ))
            .child(body)
            .focus_trap(("change-modal-focus-trap", key), focus),
    )
    .into_any_element()
}

fn presentation_rows(presentation: &ToolPresentation) -> (&str, Vec<ChangeLine>) {
    match presentation {
        ToolPresentation::Edit { path, diff, format } => (
            path,
            diff.as_deref()
                .map(|diff| parse_display_diff(diff, *format))
                .unwrap_or_else(|| {
                    vec![ChangeLine {
                        kind: ChangeKind::Ellipsis,
                        number: None,
                        content: "Preparing diff…".into(),
                    }]
                }),
        ),
        ToolPresentation::Write { path, content } => (
            path,
            content
                .lines()
                .enumerate()
                .map(|(index, line)| ChangeLine {
                    kind: ChangeKind::Addition,
                    number: u64::try_from(index)
                        .ok()
                        .and_then(|value| value.checked_add(1)),
                    content: line.to_owned(),
                })
                .collect(),
        ),
    }
}

fn change_title(path: &str) -> String {
    if path.is_empty() {
        "Changed file".into()
    } else {
        path.into()
    }
}

fn change_header(title: String, modal: bool, close: Option<AnyElement>) -> impl IntoElement {
    div()
        .min_h(if modal { px(44.0) } else { px(28.0) })
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .gap(THEME.space.sm)
        .px(if modal {
            THEME.space.md
        } else {
            THEME.space.sm
        })
        .border_b(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.surface)
        .font_family(READING_FONT_FAMILY)
        .text_size(if modal {
            THEME.type_scale.body
        } else {
            THEME.type_scale.body_small
        })
        .text_color(THEME.colors.text)
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(title),
        )
        .children(close)
}

fn preview_rows(rows: &[ChangeLine]) -> (Vec<ChangeLine>, usize) {
    (
        rows.iter().take(INLINE_PREVIEW_LINES).cloned().collect(),
        rows.len().saturating_sub(INLINE_PREVIEW_LINES),
    )
}

fn render_preview_line(row: &ChangeLine, key: usize, index: usize) -> AnyElement {
    let (marker, background, foreground) = line_colors(row.kind);
    div()
        .id(format!("tool-change-preview-{key}-{index}"))
        .w_full()
        .min_w_0()
        .h(px(24.0))
        .flex()
        .items_center()
        .overflow_hidden()
        .bg(background)
        .font_family(READING_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small)
        .child(
            div()
                .w(px(52.0))
                .flex_none()
                .flex()
                .justify_end()
                .gap(THEME.space.xs)
                .pr(THEME.space.xs)
                .border_r(THEME.border)
                .border_color(THEME.colors.border)
                .text_color(THEME.colors.subtle)
                .child(
                    row.number
                        .map_or_else(String::new, |number| number.to_string()),
                )
                .child(div().w(px(8.0)).text_center().child(marker)),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .px(THEME.space.xs)
                .text_color(foreground)
                .child(row.content.clone()),
        )
        .into_any_element()
}

fn render_edit_modal(rows: &[ChangeLine], key: usize) -> AnyElement {
    let paired = pair_edit_rows(rows);
    let truncated = paired.len() > MAX_MODAL_LINES;
    div()
        .w_full()
        .min_w_0()
        .child(
            div()
                .w_full()
                .flex()
                .border_b(THEME.border)
                .border_color(THEME.colors.border)
                .font_family(READING_FONT_FAMILY)
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.muted)
                .child(
                    div()
                        .w_1_2()
                        .px(THEME.space.sm)
                        .py(THEME.space.xs)
                        .child("OLD / DELETED"),
                )
                .child(
                    div()
                        .w_1_2()
                        .px(THEME.space.sm)
                        .py(THEME.space.xs)
                        .border_l(THEME.border)
                        .border_color(THEME.colors.border)
                        .child("NEW / ADDED"),
                ),
        )
        .children(
            paired
                .iter()
                .take(MAX_MODAL_LINES)
                .enumerate()
                .map(|(index, row)| render_paired_line(row, key, index)),
        )
        .when(truncated, |body| {
            body.child(modal_limit_hint(paired.len() - MAX_MODAL_LINES))
        })
        .into_any_element()
}

fn render_write_modal(rows: &[ChangeLine], key: usize) -> AnyElement {
    let truncated = rows.len() > MAX_MODAL_LINES;
    div()
        .w_full()
        .min_w_0()
        .children(
            rows.iter()
                .take(MAX_MODAL_LINES)
                .enumerate()
                .map(|(index, row)| {
                    let side = SideLine {
                        kind: row.kind,
                        number: row.number,
                        content: row.content.clone(),
                    };
                    render_modal_side(Some(&side), key, index, "write")
                }),
        )
        .when(truncated, |body| {
            body.child(modal_limit_hint(rows.len() - MAX_MODAL_LINES))
        })
        .into_any_element()
}

fn modal_limit_hint(remaining: usize) -> impl IntoElement {
    div()
        .px(THEME.space.md)
        .py(THEME.space.sm)
        .border_t(THEME.border)
        .border_color(THEME.colors.border)
        .text_size(THEME.type_scale.caption)
        .text_color(THEME.colors.warning)
        .child(format!("{remaining} additional lines omitted"))
}

fn render_paired_line(row: &PairedLine, key: usize, index: usize) -> AnyElement {
    div()
        .w_full()
        .min_w_0()
        .flex()
        .items_stretch()
        .border_b(THEME.border)
        .border_color(THEME.colors.border)
        .child(div().w_1_2().min_w_0().child(render_modal_side(
            row.old.as_ref(),
            key,
            index,
            "old",
        )))
        .child(
            div()
                .w_1_2()
                .min_w_0()
                .border_l(THEME.border)
                .border_color(THEME.colors.border)
                .child(render_modal_side(row.new.as_ref(), key, index, "new")),
        )
        .into_any_element()
}

fn render_modal_side(
    line: Option<&SideLine>,
    key: usize,
    index: usize,
    side: &'static str,
) -> AnyElement {
    let Some(line) = line else {
        return div()
            .min_h(px(28.0))
            .w_full()
            .bg(THEME.colors.canvas)
            .into_any_element();
    };
    let (marker, background, foreground) = line_colors(line.kind);
    div()
        .w_full()
        .min_w_0()
        .min_h(px(28.0))
        .flex()
        .items_start()
        .bg(background)
        .font_family(READING_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small)
        .child(
            div()
                .w(px(48.0))
                .flex_none()
                .flex()
                .justify_end()
                .gap(THEME.space.xs)
                .px(THEME.space.xs)
                .py(THEME.space.xs)
                .text_color(THEME.colors.subtle)
                .child(
                    line.number
                        .map_or_else(String::new, |number| number.to_string()),
                )
                .child(div().w(px(8.0)).text_center().child(marker)),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .px(THEME.space.xs)
                .py(px(2.0))
                .text_color(foreground)
                .child(code_line(
                    format!("change-modal-{key}-{index}-{side}"),
                    &soft_wrap_source(&line.content),
                    "text",
                    true,
                )),
        )
        .into_any_element()
}

fn line_colors(kind: ChangeKind) -> (&'static str, gpui::Rgba, gpui::Rgba) {
    match kind {
        ChangeKind::Context => (" ", THEME.colors.canvas, THEME.colors.text),
        ChangeKind::Addition => ("+", THEME.colors.diff_added, THEME.colors.success),
        ChangeKind::Deletion => ("-", THEME.colors.diff_deleted, THEME.colors.error),
        ChangeKind::Ellipsis => ("", THEME.colors.surface, THEME.colors.subtle),
    }
}

fn code_line(
    id: impl Into<gpui::ElementId>,
    content: &str,
    language: &str,
    wrap: bool,
) -> TextView {
    TextView::markdown(id, fenced_line(content, language))
        .style(code_line_style())
        .selectable(true)
        .when(wrap, |text| text.whitespace_normal())
        .when(!wrap, |text| text.whitespace_nowrap())
        .w_full()
        .min_w_0()
        .text_size(THEME.type_scale.body_small)
        .line_height(THEME.type_scale.line_body)
}

fn code_line_style() -> TextViewStyle {
    let mut code_block = StyleRefinement::default()
        .p_0()
        .rounded(px(0.0))
        .bg(transparent_black());
    code_block.overflow.x = Some(Overflow::Hidden);
    TextViewStyle {
        paragraph_gap: rems(0.0),
        heading_base_font_size: THEME.type_scale.body_small,
        highlight_theme: HighlightTheme::default_dark(),
        code_block,
        is_dark: true,
        ..TextViewStyle::default()
    }
}

fn pair_edit_rows(rows: &[ChangeLine]) -> Vec<PairedLine> {
    let mut paired = Vec::new();
    let mut deletions = Vec::new();
    let mut additions = Vec::new();
    let mut old_number = 1_u64;
    let mut new_number = 1_u64;
    let mut line_delta = 0_i64;

    for row in rows {
        match row.kind {
            ChangeKind::Deletion => {
                deletions.push(numbered_side(row, &mut old_number));
                line_delta = line_delta.saturating_sub(1);
            }
            ChangeKind::Addition => {
                additions.push(numbered_side(row, &mut new_number));
                line_delta = line_delta.saturating_add(1);
            }
            ChangeKind::Context => {
                flush_changes(&mut paired, &mut deletions, &mut additions);
                let old = numbered_side(row, &mut old_number);
                let new = if let Some(number) = row.number {
                    let number = apply_line_delta(number, line_delta);
                    new_number = number.saturating_add(1);
                    SideLine {
                        kind: row.kind,
                        number: Some(number),
                        content: row.content.clone(),
                    }
                } else {
                    numbered_side(row, &mut new_number)
                };
                paired.push(PairedLine {
                    old: Some(old),
                    new: Some(new),
                });
            }
            ChangeKind::Ellipsis => {
                flush_changes(&mut paired, &mut deletions, &mut additions);
                let side = SideLine {
                    kind: ChangeKind::Ellipsis,
                    number: None,
                    content: row.content.clone(),
                };
                paired.push(PairedLine {
                    old: Some(side.clone()),
                    new: Some(side),
                });
            }
        }
    }
    flush_changes(&mut paired, &mut deletions, &mut additions);
    paired
}

fn apply_line_delta(number: u64, delta: i64) -> u64 {
    if delta < 0 {
        number.saturating_sub(delta.unsigned_abs())
    } else {
        number.saturating_add(delta.unsigned_abs())
    }
}

fn numbered_side(row: &ChangeLine, next: &mut u64) -> SideLine {
    let number = row.number.unwrap_or(*next);
    *next = number.saturating_add(1);
    SideLine {
        kind: row.kind,
        number: Some(number),
        content: row.content.clone(),
    }
}

fn flush_changes(
    paired: &mut Vec<PairedLine>,
    deletions: &mut Vec<SideLine>,
    additions: &mut Vec<SideLine>,
) {
    let count = deletions.len().max(additions.len());
    for index in 0..count {
        paired.push(PairedLine {
            old: deletions.get(index).cloned(),
            new: additions.get(index).cloned(),
        });
    }
    deletions.clear();
    additions.clear();
}

fn soft_wrap_source(content: &str) -> String {
    let mut wrapped = String::with_capacity(content.len());
    let mut run_length = 0;
    for character in content.chars() {
        wrapped.push(character);
        if character.is_whitespace() {
            run_length = 0;
            continue;
        }
        run_length += 1;
        if run_length == SOFT_WRAP_COLUMNS {
            wrapped.push('\u{200b}');
            run_length = 0;
        }
    }
    wrapped
}

fn fenced_line(content: &str, language: &str) -> String {
    let fence_len = content
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
        .max(3);
    let fence = "`".repeat(fence_len);
    format!("{fence}{language}\n{content}\n{fence}")
}

fn parse_display_diff(diff: &str, format: EditDiffFormat) -> Vec<ChangeLine> {
    diff.lines()
        .map(|line| parse_display_line(line, format))
        .collect()
}

fn parse_display_line(line: &str, format: EditDiffFormat) -> ChangeLine {
    let (kind, rest) = match line.as_bytes().first().copied() {
        Some(b'+') => (ChangeKind::Addition, &line[1..]),
        Some(b'-') => (ChangeKind::Deletion, &line[1..]),
        Some(b' ') => (ChangeKind::Context, &line[1..]),
        _ => (ChangeKind::Context, line),
    };
    if rest.trim() == "..." {
        return ChangeLine {
            kind: ChangeKind::Ellipsis,
            number: None,
            content: "…".into(),
        };
    }
    if format == EditDiffFormat::Unnumbered {
        return ChangeLine {
            kind,
            number: None,
            content: rest.strip_prefix(' ').unwrap_or(rest).to_owned(),
        };
    }

    let number_start = rest.bytes().take_while(|byte| *byte == b' ').count();
    let digit_count = rest[number_start..]
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    let number_end = number_start + digit_count;
    if digit_count == 0 || rest.as_bytes().get(number_end) != Some(&b' ') {
        return ChangeLine {
            kind,
            number: None,
            content: rest.strip_prefix(' ').unwrap_or(rest).to_owned(),
        };
    }
    ChangeLine {
        kind,
        number: rest[number_start..number_end].parse().ok(),
        content: rest[number_end + 1..].to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_preview_is_fixed_at_four_rows_with_a_remaining_count() {
        let rows = parse_display_diff(
            "- one\n+ two\n three\n four\n+ five\n+ six",
            EditDiffFormat::Unnumbered,
        );
        let (preview, remaining) = preview_rows(&rows);

        assert_eq!(preview.len(), 4);
        assert_eq!(remaining, 2);
    }

    #[test]
    fn edit_rows_pair_deletions_and_additions_with_independent_numbers() {
        let rows = parse_display_diff(
            " 10 context\n- 11 old one\n- 12 old two\n+ 11 new one\n 13 tail",
            EditDiffFormat::Numbered,
        );
        let paired = pair_edit_rows(&rows);

        assert_eq!(
            paired[0].old.as_ref().and_then(|line| line.number),
            Some(10)
        );
        assert_eq!(
            paired[0].new.as_ref().and_then(|line| line.number),
            Some(10)
        );
        assert_eq!(
            paired[1].old.as_ref().and_then(|line| line.number),
            Some(11)
        );
        assert_eq!(
            paired[1].new.as_ref().and_then(|line| line.number),
            Some(11)
        );
        assert_eq!(
            paired[2].old.as_ref().and_then(|line| line.number),
            Some(12)
        );
        assert!(paired[2].new.is_none());
        assert_eq!(
            paired[3].old.as_ref().and_then(|line| line.number),
            Some(13)
        );
        assert_eq!(
            paired[3].new.as_ref().and_then(|line| line.number),
            Some(12)
        );
    }

    #[test]
    fn modal_source_inserts_soft_wrap_opportunities_without_changing_source() {
        let source = format!(
            "{} short words {}",
            "x".repeat(SOFT_WRAP_COLUMNS * 2 + 1),
            "y".repeat(SOFT_WRAP_COLUMNS + 1)
        );
        let wrapped = soft_wrap_source(&source);
        assert_eq!(wrapped.matches('\u{200b}').count(), 3);
        assert_eq!(wrapped.replace('\u{200b}', ""), source);
        assert!(!soft_wrap_source("short words reset naturally").contains('\u{200b}'));
    }

    #[test]
    fn parses_pi_display_diff_lines() {
        assert_eq!(
            parse_display_line("- 57   let pending = true;", EditDiffFormat::Numbered),
            ChangeLine {
                kind: ChangeKind::Deletion,
                number: Some(57),
                content: "  let pending = true;".into(),
            }
        );
        assert_eq!(
            parse_display_line("-  7   padded = true;", EditDiffFormat::Numbered),
            ChangeLine {
                kind: ChangeKind::Deletion,
                number: Some(7),
                content: "  padded = true;".into(),
            }
        );
        assert_eq!(
            parse_display_line("+105     terminalFocused = true;", EditDiffFormat::Numbered,),
            ChangeLine {
                kind: ChangeKind::Addition,
                number: Some(105),
                content: "    terminalFocused = true;".into(),
            }
        );
        assert_eq!(
            parse_display_line("     ...", EditDiffFormat::Numbered).kind,
            ChangeKind::Ellipsis
        );
    }

    #[test]
    fn edit_argument_previews_without_numbers_remain_code() {
        assert_eq!(
            parse_display_line("- 123 source", EditDiffFormat::Unnumbered),
            ChangeLine {
                kind: ChangeKind::Deletion,
                number: None,
                content: "123 source".into(),
            }
        );
    }

    #[test]
    fn open_modal_resolves_authoritative_tool_presentation_by_call_id() {
        let preview = ToolPresentation::Edit {
            path: "src/lib.rs".into(),
            diff: Some("- old\n+ new".into()),
            format: EditDiffFormat::Unnumbered,
        };
        let authoritative = ToolPresentation::Edit {
            path: "src/lib.rs".into(),
            diff: Some("- 9 old\n+ 9 new".into()),
            format: EditDiffFormat::Numbered,
        };
        let modal = ChangeModal {
            presentation: preview,
            tool_call_id: Some("edit-1".into()),
            key: 1,
        };
        let items = vec![Arc::new(TranscriptItem {
            kind: crate::conversation::TranscriptKind::Tool,
            label: "Edit".into(),
            text: String::new(),
            streaming: false,
            is_error: false,
            tool_call_id: Some("edit-1".into()),
            tool_output: String::new(),
            tool_presentation: Some(authoritative.clone()),
        })];

        assert_eq!(modal_presentation(&modal, &items), &authoritative);
    }

    #[test]
    fn code_fence_grows_past_content_backticks() {
        assert!(fenced_line("```", "text").starts_with("````text\n"));
    }
}
