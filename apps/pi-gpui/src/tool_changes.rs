//! Native, selectable presentations for file mutation tools.

use std::{path::Path, rc::Rc};

use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, StyleRefinement, Styled as _, Window, div,
    prelude::FluentBuilder as _, px, rems, transparent_black,
};
use gpui_component::{
    Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    highlighter::HighlightTheme,
    text::{TextView, TextViewStyle},
};

use crate::{
    conversation::{EditDiffFormat, ToolPresentation},
    theme::{MONO_FONT_FAMILY, THEME},
};

const MAX_DIFF_LINES: usize = 140;

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
struct SideLine {
    kind: ChangeKind,
    number: Option<u64>,
    content: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PairedLine {
    old: Option<SideLine>,
    new: Option<SideLine>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PreparedToolChange {
    Edit(Vec<PairedLine>),
    Write(Vec<ChangeLine>),
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
                PreparedToolChange::Edit(pair_edit_rows(&rows))
            }),
        ),
        ToolPresentation::Write {
            path,
            content,
            prepared,
        } => (
            path,
            prepared.get_or_init(|| PreparedToolChange::Write(write_rows(content))),
        ),
    };
    let language = language_for_path(path);
    let mode = match prepared {
        PreparedToolChange::Edit(_) => requested_mode,
        PreparedToolChange::Write(_) => EmbeddedDiffMode::Unified,
    };
    let metadata = diff_metadata(path, prepared, mode);
    let body = match prepared {
        PreparedToolChange::Edit(rows) => render_edit_diff(rows, &language, key, mode),
        PreparedToolChange::Write(rows) => render_write_diff(rows, &language, key),
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
        PreparedToolChange::Edit(rows) => rows.iter().fold((0, 0), |counts, row| {
            (
                counts.0
                    + usize::from(
                        row.new
                            .as_ref()
                            .is_some_and(|line| line.kind == ChangeKind::Addition),
                    ),
                counts.1
                    + usize::from(
                        row.old
                            .as_ref()
                            .is_some_and(|line| line.kind == ChangeKind::Deletion),
                    ),
            )
        }),
        PreparedToolChange::Write(rows) => (
            rows.iter()
                .filter(|line| line.kind == ChangeKind::Addition)
                .count(),
            0,
        ),
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

fn write_rows(content: &str) -> Vec<ChangeLine> {
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
        .collect()
}

fn render_edit_diff(
    paired: &[PairedLine],
    language: &str,
    key: usize,
    mode: EmbeddedDiffMode,
) -> AnyElement {
    if mode == EmbeddedDiffMode::Unified {
        return render_unified_edit_diff(paired, language, key);
    }
    let truncated = paired.len() > MAX_DIFF_LINES;
    div()
        .id(("split-diff-body", key))
        .w_full()
        .min_w_0()
        .max_h(px(360.0))
        .overflow_scroll()
        .children(
            paired
                .iter()
                .take(MAX_DIFF_LINES)
                .enumerate()
                .map(|(index, row)| render_paired_line(row, language, key, index)),
        )
        .when(truncated, |body| {
            body.child(modal_limit_hint(paired.len() - MAX_DIFF_LINES))
        })
        .into_any_element()
}

fn render_unified_edit_diff(paired: &[PairedLine], language: &str, key: usize) -> AnyElement {
    let rows = unified_edit_rows(paired);
    let truncated = rows.len() > MAX_DIFF_LINES;
    div()
        .id(("unified-diff-body", key))
        .w_full()
        .min_w_0()
        .max_h(px(360.0))
        .overflow_scroll()
        .children(
            rows.iter()
                .take(MAX_DIFF_LINES)
                .enumerate()
                .map(|(index, row)| render_diff_side(Some(row), language, key, index, "unified")),
        )
        .when(truncated, |body| {
            body.child(modal_limit_hint(rows.len() - MAX_DIFF_LINES))
        })
        .into_any_element()
}

fn unified_edit_rows(paired: &[PairedLine]) -> Vec<SideLine> {
    let mut rows = Vec::new();
    for pair in paired {
        match (&pair.old, &pair.new) {
            (Some(old), Some(new))
                if old.kind == new.kind
                    && matches!(old.kind, ChangeKind::Context | ChangeKind::Ellipsis) =>
            {
                rows.push(old.clone());
            }
            (Some(old), Some(new)) => {
                rows.push(old.clone());
                rows.push(new.clone());
            }
            (Some(old), None) => rows.push(old.clone()),
            (None, Some(new)) => rows.push(new.clone()),
            (None, None) => {}
        }
    }
    rows
}

fn render_write_diff(rows: &[ChangeLine], language: &str, key: usize) -> AnyElement {
    let truncated = rows.len() > MAX_DIFF_LINES;
    div()
        .id(("write-diff-body", key))
        .w_full()
        .min_w_0()
        .max_h(px(360.0))
        .overflow_scroll()
        .children(
            rows.iter()
                .take(MAX_DIFF_LINES)
                .enumerate()
                .map(|(index, row)| {
                    let side = SideLine {
                        kind: row.kind,
                        number: row.number,
                        content: row.content.clone(),
                    };
                    render_diff_side(Some(&side), language, key, index, "write")
                }),
        )
        .when(truncated, |body| {
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

fn render_paired_line(row: &PairedLine, language: &str, key: usize, index: usize) -> AnyElement {
    div()
        .w_full()
        .min_w_0()
        .flex()
        .items_stretch()
        .child(div().w_1_2().min_w_0().child(render_diff_side(
            row.old.as_ref(),
            language,
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
                .child(render_diff_side(
                    row.new.as_ref(),
                    language,
                    key,
                    index,
                    "new",
                )),
        )
        .into_any_element()
}

fn render_diff_side(
    line: Option<&SideLine>,
    language: &str,
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
    let line_number = gutter_label(marker, line.number);
    div()
        .w_full()
        .min_w_0()
        .min_h(px(28.0))
        .flex()
        .items_start()
        .bg(background)
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small)
        .child(
            div()
                .w(px(48.0))
                .flex_none()
                .px(THEME.space.xs)
                .py(THEME.space.xs)
                .text_align(gpui::TextAlign::Right)
                .text_color(if line.kind == ChangeKind::Context {
                    THEME.colors.subtle
                } else {
                    foreground
                })
                .child(line_number),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .px(THEME.space.xs)
                .py(px(2.0))
                .text_color(foreground)
                .child(code_line(
                    format!("change-diff-{key}-{index}-{side}"),
                    &replace_tabs(&line.content),
                    language,
                )),
        )
        .into_any_element()
}

fn gutter_label(marker: &str, number: Option<u64>) -> String {
    match (marker, number) {
        ("", None) => String::new(),
        (marker, None) => marker.to_owned(),
        (marker, Some(number)) => format!("{marker}{number}"),
    }
}

fn line_colors(kind: ChangeKind) -> (&'static str, gpui::Rgba, gpui::Rgba) {
    match kind {
        ChangeKind::Context => (" ", THEME.colors.canvas, THEME.colors.text),
        ChangeKind::Addition => ("+", THEME.colors.diff_added, THEME.colors.success),
        ChangeKind::Deletion => ("-", THEME.colors.diff_deleted, THEME.colors.error),
        ChangeKind::Ellipsis => ("", THEME.colors.surface, THEME.colors.subtle),
    }
}

fn code_line(id: impl Into<gpui::ElementId>, content: &str, language: &str) -> TextView {
    TextView::markdown(id, fenced_line(content, language))
        .style(code_line_style())
        .selectable(true)
        .whitespace_nowrap()
        .w_full()
        .min_w_0()
        .text_size(THEME.type_scale.body_small)
        .line_height(THEME.type_scale.line_body)
}

fn code_line_style() -> TextViewStyle {
    let code_block = StyleRefinement::default()
        .p_0()
        .rounded(px(0.0))
        .bg(transparent_black());
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

fn replace_tabs(content: &str) -> String {
    content.replace('\t', "   ")
}

fn language_for_path(path: &str) -> String {
    let path = Path::new(path);
    match path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
    {
        "Dockerfile" => "dockerfile".into(),
        "Makefile" | "GNUmakefile" => "make".into(),
        _ => path
            .extension()
            .and_then(|extension| extension.to_str())
            .filter(|extension| !extension.is_empty())
            .unwrap_or("text")
            .to_ascii_lowercase(),
    }
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
    fn diff_source_matches_terminal_tab_width() {
        assert_eq!(replace_tabs("before\tafter"), "before   after");
    }

    #[test]
    fn file_paths_select_the_syntax_language() {
        assert_eq!(language_for_path("src/main.ts"), "ts");
        assert_eq!(language_for_path("Dockerfile"), "dockerfile");
        assert_eq!(language_for_path("Makefile"), "make");
        assert_eq!(language_for_path("LICENSE"), "text");
    }

    #[test]
    fn metadata_counts_changes_and_reports_the_actual_view() {
        let rows = parse_display_diff(
            " same\n- old one\n- old two\n+ new one",
            EditDiffFormat::Unnumbered,
        );
        let prepared = PreparedToolChange::Edit(pair_edit_rows(&rows));

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
        let unified = unified_edit_rows(&pair_edit_rows(&rows));

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

    #[test]
    fn code_fence_grows_past_content_backticks() {
        assert!(fenced_line("```", "text").starts_with("````text\n"));
    }
}
