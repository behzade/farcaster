//! Native presentations for file mutation tools.

use std::{rc::Rc, sync::Arc};

use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Sizable as _, Size,
    button::{Button, ButtonVariants as _},
};

use crate::{
    conversation::{EditDiffFormat, ToolPresentation},
    diff_element::{DiffCell, DiffElement, DiffPaintRow, DiffTone},
    syntax_highlight::{HighlightedText, highlight_lines, language_for_path},
    theme::{MONO_FONT_FAMILY, THEME},
};

// Keep every embedded edit and write preview bounded; Expand shows the full diff.
const MAX_DIFF_LINES: usize = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EmbeddedDiffMode {
    Split,
    Unified,
}

pub(crate) type ExpandHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChangeKind {
    Context,
    Addition,
    Deletion,
    Ellipsis,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChangeLine {
    kind: ChangeKind,
    number: Option<u64>,
    content: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SideLine {
    kind: ChangeKind,
    number: Option<u64>,
    content: String,
    syntax: HighlightedText,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PairedLine {
    old: Option<SideLine>,
    new: Option<SideLine>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PreparedToolChange {
    Edit {
        rows: Arc<Vec<PairedLine>>,
        additions: usize,
        deletions: usize,
    },
    Write {
        rows: Arc<Vec<SideLine>>,
        additions: usize,
    },
}

pub(crate) fn render(
    presentation: &ToolPresentation,
    key: usize,
    requested_mode: EmbeddedDiffMode,
    on_expand: Option<ExpandHandler>,
) -> AnyElement {
    let (path, prepared) = match presentation {
        ToolPresentation::Edit {
            path,
            diff,
            format,
            prepared,
        } => (
            path,
            prepared.get_or_init(|| {
                let rows = diff
                    .as_deref()
                    .map_or_else(preparing_rows, |diff| parse_display_diff(diff, *format));
                let additions = rows
                    .iter()
                    .filter(|row| row.kind == ChangeKind::Addition)
                    .count();
                let deletions = rows
                    .iter()
                    .filter(|row| row.kind == ChangeKind::Deletion)
                    .count();
                PreparedToolChange::Edit {
                    rows: Arc::new(pair_edit_rows(&rows, &language_for_path(path))),
                    additions,
                    deletions,
                }
            }),
        ),
        ToolPresentation::Write {
            path,
            content,
            prepared,
        } => (
            path,
            prepared.get_or_init(|| {
                let rows = write_rows(content, &language_for_path(path));
                let additions = rows.len();
                PreparedToolChange::Write {
                    rows: Arc::new(rows),
                    additions,
                }
            }),
        ),
    };
    let mode = match prepared {
        PreparedToolChange::Edit { .. } => requested_mode,
        PreparedToolChange::Write { .. } => EmbeddedDiffMode::Unified,
    };
    let metadata = diff_metadata(path, prepared, mode);
    let body = match prepared {
        PreparedToolChange::Edit { rows, .. } => render_edit_diff(rows, mode),
        PreparedToolChange::Write { rows, .. } => render_write_diff(rows),
    };
    div()
        .id(("tool-change", key))
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .border_y(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.canvas)
        .child(render_diff_header(metadata, key, on_expand))
        .child(body)
        .into_any_element()
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DiffMetadata<'a> {
    path: &'a str,
    additions: usize,
    deletions: usize,
    mode: EmbeddedDiffMode,
}

fn diff_metadata<'a>(
    path: &'a str,
    prepared: &PreparedToolChange,
    mode: EmbeddedDiffMode,
) -> DiffMetadata<'a> {
    let (additions, deletions) = match prepared {
        PreparedToolChange::Edit {
            additions,
            deletions,
            ..
        } => (*additions, *deletions),
        PreparedToolChange::Write { additions, .. } => (*additions, 0),
    };
    DiffMetadata {
        path,
        additions,
        deletions,
        mode,
    }
}

fn render_diff_header(
    metadata: DiffMetadata<'_>,
    key: usize,
    on_expand: Option<ExpandHandler>,
) -> impl IntoElement {
    let expand = on_expand.map(|handler| {
        Button::new(("expand-tool-change", key))
            .label("Expand")
            .with_size(Size::XSmall)
            .ghost()
            .on_click(move |_, window, cx| handler(window, cx))
    });
    div()
        .w_full()
        .min_w_0()
        .h(px(34.0))
        .px(THEME.space.sm)
        .flex()
        .items_center()
        .gap(THEME.space.sm)
        .border_b(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.panel)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_family(MONO_FONT_FAMILY)
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.text)
                .child(metadata.path.to_owned()),
        )
        .child(
            div()
                .font_family(MONO_FONT_FAMILY)
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.success)
                .child(format!("+{}", metadata.additions)),
        )
        .child(
            div()
                .font_family(MONO_FONT_FAMILY)
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.error)
                .child(format!("-{}", metadata.deletions)),
        )
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.muted)
                .child(match metadata.mode {
                    EmbeddedDiffMode::Split => "Split",
                    EmbeddedDiffMode::Unified => "Unified",
                }),
        )
        .children(expand)
}

fn preparing_rows() -> Vec<ChangeLine> {
    vec![ChangeLine {
        kind: ChangeKind::Ellipsis,
        number: None,
        content: "Preparing diff…".into(),
    }]
}

fn write_rows(content: &str, language: &str) -> Vec<SideLine> {
    let mut rows = content
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let content = replace_tabs(line);
            SideLine {
                kind: ChangeKind::Addition,
                number: u64::try_from(index)
                    .ok()
                    .and_then(|value| value.checked_add(1)),
                syntax: HighlightedText::plain(content.clone()),
                content,
            }
        })
        .collect::<Vec<_>>();
    highlight_side_lines(rows.iter_mut().take(MAX_DIFF_LINES), language);
    rows
}

fn render_edit_diff(paired: &Arc<Vec<PairedLine>>, mode: EmbeddedDiffMode) -> AnyElement {
    if mode == EmbeddedDiffMode::Unified {
        return render_unified_edit_diff(paired);
    }
    let visible_count = paired.len().min(MAX_DIFF_LINES);
    let rows = paired.clone();
    div()
        .w_full()
        .min_w_0()
        .child(DiffElement::split(
            visible_count,
            px(28.0),
            px(48.0),
            move |index| {
                rows.get(index)
                    .map_or_else(DiffPaintRow::default, |row| DiffPaintRow {
                        old: row.old.as_ref().map(diff_cell),
                        new: row.new.as_ref().map(diff_cell),
                    })
            },
        ))
        .when(paired.len() > MAX_DIFF_LINES, |body| {
            body.child(modal_limit_hint(paired.len() - MAX_DIFF_LINES))
        })
        .into_any_element()
}

fn render_unified_edit_diff(paired: &Arc<Vec<PairedLine>>) -> AnyElement {
    let row_count = paired.iter().flat_map(unified_pair_rows).flatten().count();
    let rows = Arc::new(
        paired
            .iter()
            .flat_map(unified_pair_rows)
            .flatten()
            .take(MAX_DIFF_LINES)
            .cloned()
            .collect::<Vec<_>>(),
    );
    let visible_count = rows.len();
    div()
        .w_full()
        .min_w_0()
        .child(DiffElement::unified(
            visible_count,
            px(28.0),
            px(48.0),
            move |index| rows.get(index).map(diff_cell),
        ))
        .when(row_count > MAX_DIFF_LINES, |body| {
            body.child(modal_limit_hint(row_count - MAX_DIFF_LINES))
        })
        .into_any_element()
}

fn unified_pair_rows(pair: &PairedLine) -> [Option<&SideLine>; 2] {
    match (&pair.old, &pair.new) {
        (Some(old), Some(new))
            if old.kind == new.kind
                && matches!(old.kind, ChangeKind::Context | ChangeKind::Ellipsis) =>
        {
            [Some(old), None]
        }
        (old, new) => [old.as_ref(), new.as_ref()],
    }
}

#[cfg(test)]
fn unified_edit_rows(paired: &[PairedLine]) -> Vec<SideLine> {
    paired
        .iter()
        .flat_map(unified_pair_rows)
        .flatten()
        .cloned()
        .collect()
}

fn render_write_diff(rows: &Arc<Vec<SideLine>>) -> AnyElement {
    let visible_count = rows.len().min(MAX_DIFF_LINES);
    let source = rows.clone();
    div()
        .w_full()
        .min_w_0()
        .child(DiffElement::unified(
            visible_count,
            px(28.0),
            px(48.0),
            move |index| source.get(index).map(diff_cell),
        ))
        .when(rows.len() > MAX_DIFF_LINES, |body| {
            body.child(modal_limit_hint(rows.len() - MAX_DIFF_LINES))
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

fn diff_cell(line: &SideLine) -> DiffCell {
    let (marker, tone) = match line.kind {
        ChangeKind::Context => (" ", DiffTone::Context),
        ChangeKind::Addition => ("+", DiffTone::Addition),
        ChangeKind::Deletion => ("-", DiffTone::Deletion),
        ChangeKind::Ellipsis => ("", DiffTone::Muted),
    };
    DiffCell {
        gutter: Some(gutter_label(marker, line.number)),
        text: line.syntax.clone(),
        tone,
    }
}

fn gutter_label(marker: &str, number: Option<u64>) -> String {
    match (marker, number) {
        ("", None) => String::new(),
        (marker, None) => marker.to_owned(),
        (marker, Some(number)) => format!("{marker}{number}"),
    }
}

fn pair_edit_rows(rows: &[ChangeLine], language: &str) -> Vec<PairedLine> {
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
                    let content = replace_tabs(&row.content);
                    SideLine {
                        kind: row.kind,
                        number: Some(number),
                        syntax: HighlightedText::plain(content.clone()),
                        content,
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
                let content = replace_tabs(&row.content);
                let side = SideLine {
                    kind: ChangeKind::Ellipsis,
                    number: None,
                    syntax: HighlightedText::plain(content.clone()),
                    content,
                };
                paired.push(PairedLine {
                    old: Some(side.clone()),
                    new: Some(side),
                });
            }
        }
    }
    flush_changes(&mut paired, &mut deletions, &mut additions);
    highlight_side_lines(
        paired
            .iter_mut()
            .take(MAX_DIFF_LINES)
            .filter_map(|row| row.old.as_mut()),
        language,
    );
    highlight_side_lines(
        paired
            .iter_mut()
            .take(MAX_DIFF_LINES)
            .filter_map(|row| row.new.as_mut()),
        language,
    );
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
    let content = replace_tabs(&row.content);
    SideLine {
        kind: row.kind,
        number: Some(number),
        syntax: HighlightedText::plain(content.clone()),
        content,
    }
}

fn highlight_side_lines<'a>(lines: impl Iterator<Item = &'a mut SideLine>, language: &str) {
    let lines = lines.collect::<Vec<_>>();
    if lines.is_empty() {
        return;
    }
    let highlighted = {
        let source = lines
            .iter()
            .map(|line| line.content.as_str())
            .collect::<Vec<_>>();
        highlight_lines(&source, language)
    };
    for (line, syntax) in lines.into_iter().zip(highlighted) {
        line.syntax = syntax;
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

fn replace_tabs(content: &str) -> String {
    content.replace('\t', "   ")
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
    use std::sync::Arc;

    #[test]
    fn unnumbered_changes_keep_visible_non_color_markers() {
        assert_eq!(gutter_label("+", None), "+");
        assert_eq!(gutter_label("-", None), "-");
    }

    #[test]
    fn prepared_diff_rows_are_reused_across_renders() {
        let prepared = Arc::default();
        let presentation = ToolPresentation::Edit {
            path: "src/main.rs".into(),
            diff: Some("- 1 old\n+ 1 new".into()),
            format: EditDiffFormat::Numbered,
            prepared: Arc::clone(&prepared),
        };

        assert!(prepared.get().is_none());
        let _ = render(&presentation, 1, EmbeddedDiffMode::Split, None);
        let first = prepared.get().expect("render should prepare the diff") as *const _;
        let _ = render(&presentation, 1, EmbeddedDiffMode::Split, None);
        let second = prepared.get().expect("prepared diff should remain cached") as *const _;

        assert_eq!(first, second);
    }

    #[test]
    fn embedded_highlighting_stops_at_the_render_limit() {
        let content = (0..=MAX_DIFF_LINES)
            .map(|index| format!("let value_{index} = {index};"))
            .collect::<Vec<_>>()
            .join("\n");
        let rows = write_rows(&content, "rs");

        assert!(
            rows[..MAX_DIFF_LINES]
                .iter()
                .all(|row| row.syntax.has_highlights())
        );
        assert!(!rows[MAX_DIFF_LINES].syntax.has_highlights());
    }

    #[test]
    fn edit_rows_pair_deletions_and_additions_with_independent_numbers() {
        let rows = parse_display_diff(
            " 10 context\n- 11 old one\n- 12 old two\n+ 11 new one\n 13 tail",
            EditDiffFormat::Numbered,
        );
        let paired = pair_edit_rows(&rows, "rs");

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
    fn diff_source_matches_terminal_tab_width() {
        assert_eq!(replace_tabs("before\tafter"), "before   after");
    }

    #[test]
    fn metadata_counts_changes_and_reports_the_actual_view() {
        let rows = parse_display_diff(
            " same\n- old one\n- old two\n+ new one",
            EditDiffFormat::Unnumbered,
        );
        let prepared = PreparedToolChange::Edit {
            rows: Arc::new(pair_edit_rows(&rows, "rs")),
            additions: 1,
            deletions: 2,
        };

        assert_eq!(
            diff_metadata("src/main.rs", &prepared, EmbeddedDiffMode::Unified),
            DiffMetadata {
                path: "src/main.rs",
                additions: 1,
                deletions: 2,
                mode: EmbeddedDiffMode::Unified,
            }
        );
    }

    #[test]
    fn unified_rows_preserve_change_order_without_duplicating_context() {
        let rows = parse_display_diff(" before\n- old\n+ new\n after", EditDiffFormat::Unnumbered);
        let unified = unified_edit_rows(&pair_edit_rows(&rows, "rs"));

        assert_eq!(unified.len(), 4);
        assert_eq!(unified[0].kind, ChangeKind::Context);
        assert_eq!(unified[1].kind, ChangeKind::Deletion);
        assert_eq!(unified[2].kind, ChangeKind::Addition);
        assert_eq!(unified[3].kind, ChangeKind::Context);
        assert!(
            unified
                .iter()
                .all(|line| !line.content.contains('\u{200b}'))
        );
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
}
