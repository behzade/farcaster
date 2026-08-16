//! Native, selectable presentations for file mutation tools.

use std::{path::Path, sync::Arc};

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ListHorizontalSizingBehavior,
    ListSizingBehavior, Overflow, ParentElement as _, StyleRefinement, Styled as _, div,
    prelude::FluentBuilder as _, px, rems, transparent_black, uniform_list,
};
use gpui_component::{
    highlighter::HighlightTheme,
    text::{TextView, TextViewStyle},
};

use crate::{
    conversation::ToolPresentation,
    theme::{READING_FONT_FAMILY, THEME},
};

const MAX_RENDERED_LINES: usize = 600;

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

pub(crate) fn render(presentation: &ToolPresentation, key: usize) -> AnyElement {
    match presentation {
        ToolPresentation::Edit { path, diff } => render_edit(path, diff.as_deref(), key),
        ToolPresentation::Write { path, content } => render_write(path, content, key),
    }
}

fn render_edit(path: &str, diff: Option<&str>, key: usize) -> AnyElement {
    let rows = diff.map(parse_display_diff).unwrap_or_else(|| {
        vec![ChangeLine {
            kind: ChangeKind::Ellipsis,
            number: None,
            content: "Preparing diff…".into(),
        }]
    });
    render_change(path, rows, key)
}

fn render_write(path: &str, content: &str, key: usize) -> AnyElement {
    let rows = content
        .lines()
        .enumerate()
        .map(|(index, line)| ChangeLine {
            kind: ChangeKind::Addition,
            number: u64::try_from(index)
                .ok()
                .and_then(|value| value.checked_add(1)),
            content: line.to_owned(),
        })
        .collect();
    render_change(path, rows, key)
}

fn render_change(path: &str, rows: Vec<ChangeLine>, key: usize) -> AnyElement {
    let language = language_for_path(path);
    let title = if path.is_empty() {
        "Changed file".to_owned()
    } else {
        path.to_owned()
    };
    let truncated = rows.len() > MAX_RENDERED_LINES;
    let displayed = Arc::new(
        rows.into_iter()
            .take(MAX_RENDERED_LINES)
            .collect::<Vec<_>>(),
    );
    let widest = displayed
        .iter()
        .enumerate()
        .max_by_key(|(_, row)| row.content.len())
        .map(|(index, _)| index);
    let row_count = displayed.len();
    let list_rows = displayed;
    let body = uniform_list(("tool-change-lines", key), row_count, move |range, _, _| {
        range
            .filter_map(|index| {
                list_rows
                    .get(index)
                    .map(|row| render_line(row, &language, key, index))
            })
            .collect::<Vec<_>>()
    })
    .with_width_from_item(widest)
    .with_sizing_behavior(ListSizingBehavior::Infer)
    .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
    .w_full()
    .max_h(px(420.0));
    div()
        .id(("tool-change", key))
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .rounded(THEME.radius)
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.canvas)
        .child(
            div()
                .h(px(28.0))
                .w_full()
                .flex()
                .items_center()
                .px(THEME.space.sm)
                .border_b(THEME.border)
                .border_color(THEME.colors.border)
                .bg(THEME.colors.surface)
                .font_family(READING_FONT_FAMILY)
                .text_size(THEME.type_scale.body_small)
                .text_color(THEME.colors.text)
                .child(title),
        )
        .child(body)
        .when(truncated, |view| {
            view.child(
                div()
                    .px(THEME.space.sm)
                    .py(THEME.space.xs)
                    .border_t(THEME.border)
                    .border_color(THEME.colors.border)
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.warning)
                    .child(format!("Preview limited to {MAX_RENDERED_LINES} lines")),
            )
        })
        .into_any_element()
}

fn render_line(row: &ChangeLine, language: &str, key: usize, index: usize) -> AnyElement {
    let (marker, background, foreground) = match row.kind {
        ChangeKind::Context => (" ", THEME.colors.canvas, THEME.colors.text),
        ChangeKind::Addition => ("+", THEME.colors.diff_added, THEME.colors.success),
        ChangeKind::Deletion => ("-", THEME.colors.diff_deleted, THEME.colors.error),
        ChangeKind::Ellipsis => ("", THEME.colors.surface, THEME.colors.subtle),
    };
    div()
        .w_full()
        .min_w_full()
        .h(px(24.0))
        .flex_none()
        .flex()
        .items_center()
        .whitespace_nowrap()
        .bg(background)
        .font_family(READING_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small)
        .line_height(px(20.0))
        .child(
            div()
                .w(px(52.0))
                .min_h(px(24.0))
                .flex_none()
                .flex()
                .items_center()
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
                .flex_none()
                .px(THEME.space.xs)
                .py(px(2.0))
                .text_color(foreground)
                .child(code_line(
                    format!("tool-change-line-{key}-{index}"),
                    &row.content,
                    language,
                )),
        )
        .into_any_element()
}

fn code_line(id: impl Into<gpui::ElementId>, content: &str, language: &str) -> TextView {
    TextView::markdown(id, fenced_line(content, language))
        .style(code_line_style())
        .selectable(true)
        .whitespace_nowrap()
        .text_size(THEME.type_scale.body_small)
        .line_height(THEME.type_scale.line_body)
}

fn code_line_style() -> TextViewStyle {
    let mut code_block = StyleRefinement::default()
        .p_0()
        .rounded(px(0.0))
        .bg(transparent_black());
    code_block.overflow.x = Some(Overflow::Visible);
    TextViewStyle {
        paragraph_gap: rems(0.0),
        heading_base_font_size: THEME.type_scale.body_small,
        highlight_theme: HighlightTheme::default_dark(),
        code_block,
        is_dark: true,
        ..TextViewStyle::default()
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

fn parse_display_diff(diff: &str) -> Vec<ChangeLine> {
    diff.lines().map(parse_display_line).collect()
}

fn parse_display_line(line: &str) -> ChangeLine {
    let (kind, rest) = match line.as_bytes().first().copied() {
        Some(b'+') => (ChangeKind::Addition, &line[1..]),
        Some(b'-') => (ChangeKind::Deletion, &line[1..]),
        Some(b' ') => (ChangeKind::Context, &line[1..]),
        _ => (ChangeKind::Context, line),
    };
    let trimmed = rest.trim_start();
    if trimmed == "..." {
        return ChangeLine {
            kind: ChangeKind::Ellipsis,
            number: None,
            content: "…".into(),
        };
    }
    let digit_count = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    let has_number_separator = digit_count > 0
        && trimmed
            .as_bytes()
            .get(digit_count)
            .is_some_and(u8::is_ascii_whitespace);
    if !has_number_separator {
        return ChangeLine {
            kind,
            number: None,
            content: rest.strip_prefix(' ').unwrap_or(rest).to_owned(),
        };
    }
    ChangeLine {
        kind,
        number: trimmed[..digit_count].parse().ok(),
        content: trimmed[digit_count..].trim_start_matches(' ').to_owned(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pi_display_diff_lines() {
        assert_eq!(
            parse_display_line("- 57   let pending = true;"),
            ChangeLine {
                kind: ChangeKind::Deletion,
                number: Some(57),
                content: "let pending = true;".into(),
            }
        );
        assert_eq!(
            parse_display_line("+105     terminalFocused = true;"),
            ChangeLine {
                kind: ChangeKind::Addition,
                number: Some(105),
                content: "terminalFocused = true;".into(),
            }
        );
        assert_eq!(parse_display_line("     ...").kind, ChangeKind::Ellipsis);
    }

    #[test]
    fn edit_argument_previews_without_numbers_remain_code() {
        assert_eq!(
            parse_display_line("- old"),
            ChangeLine {
                kind: ChangeKind::Deletion,
                number: None,
                content: "old".into(),
            }
        );
    }

    #[test]
    fn code_fence_grows_past_content_backticks() {
        assert!(fenced_line("```", "text").starts_with("````text\n"));
    }
}
