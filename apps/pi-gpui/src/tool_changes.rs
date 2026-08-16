//! Native, selectable presentations for file mutation tools.

use std::path::Path;

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement, Overflow, ParentElement as _,
    StyleRefinement, Styled as _, div, prelude::FluentBuilder as _, px, rems, transparent_black,
};
use gpui_component::{
    highlighter::HighlightTheme,
    text::{TextView, TextViewStyle},
};

use crate::{
    conversation::{EditDiffFormat, ToolPresentation},
    theme::{READING_FONT_FAMILY, THEME},
};

const MAX_DIFF_LINES: usize = 600;
const SOFT_WRAP_COLUMNS: usize = 12;

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

pub(crate) fn render(presentation: &ToolPresentation, key: usize) -> AnyElement {
    let (path, rows) = presentation_rows(presentation);
    let language = language_for_path(path);
    let body = match presentation {
        ToolPresentation::Edit { .. } => render_edit_diff(&rows, &language, key),
        ToolPresentation::Write { .. } => render_write_diff(&rows, &language, key),
    };
    div()
        .id(("tool-change", key))
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .border_y(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.canvas)
        .child(body)
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

fn render_edit_diff(rows: &[ChangeLine], language: &str, key: usize) -> AnyElement {
    let paired = pair_edit_rows(rows);
    let truncated = paired.len() > MAX_DIFF_LINES;
    div()
        .w_full()
        .min_w_0()
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

fn render_write_diff(rows: &[ChangeLine], language: &str, key: usize) -> AnyElement {
    let truncated = rows.len() > MAX_DIFF_LINES;
    div()
        .w_full()
        .min_w_0()
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
    let line_number = line.number.map_or_else(String::new, |number| {
        if marker.is_empty() || marker == " " {
            number.to_string()
        } else {
            format!("{marker}{number}")
        }
    });
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
                .overflow_hidden()
                .px(THEME.space.xs)
                .py(px(2.0))
                .text_color(foreground)
                .child(code_line(
                    format!("change-diff-{key}-{index}-{side}"),
                    &soft_wrap_source(&replace_tabs(&line.content)),
                    language,
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
    fn diff_source_inserts_soft_wrap_opportunities_without_changing_source() {
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
    fn code_fence_grows_past_content_backticks() {
        assert!(fenced_line("```", "text").starts_with("````text\n"));
    }
}
