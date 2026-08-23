//! Bounded parsing and syntax enrichment for embedded file-tool previews.

use std::sync::Arc;

use crate::{
    conversation::EditDiffFormat,
    syntax_highlight::{HighlightedText, highlight_lines, language_for_path},
};

const MAX_LINES: usize = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ChangeKind {
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
pub(super) struct SideLine {
    pub(super) kind: ChangeKind,
    pub(super) number: Option<u64>,
    pub(super) syntax: HighlightedText,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PairedLine {
    pub(super) old: Option<SideLine>,
    pub(super) new: Option<SideLine>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedToolChange {
    kind: PreparedKind,
    additions: usize,
    deletions: usize,
    omitted: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PreparedKind {
    Edit {
        split: Arc<Vec<PairedLine>>,
        unified: Arc<Vec<SideLine>>,
    },
    Write(Arc<Vec<SideLine>>),
}

impl PreparedToolChange {
    pub(super) fn split_rows(&self) -> Option<&Arc<Vec<PairedLine>>> {
        match &self.kind {
            PreparedKind::Edit { split, .. } => Some(split),
            PreparedKind::Write(_) => None,
        }
    }

    pub(super) fn unified_rows(&self) -> &Arc<Vec<SideLine>> {
        match &self.kind {
            PreparedKind::Edit { unified, .. } => unified,
            PreparedKind::Write(rows) => rows,
        }
    }

    pub(super) fn counts(&self) -> (usize, usize) {
        (self.additions, self.deletions)
    }

    pub(super) fn omitted(&self) -> usize {
        self.omitted
    }
}

pub(crate) fn prepare_edit(
    path: &str,
    diff: Option<&str>,
    format: EditDiffFormat,
) -> PreparedToolChange {
    let _timing = crate::performance::OperationTiming::new(
        crate::performance::OperationKind::ToolPreview,
        diff.map_or(0, str::len),
    );
    let language = language_for_path(path);
    let Some(diff) = diff else {
        let row = ChangeLine {
            kind: ChangeKind::Ellipsis,
            number: None,
            content: "Preparing diff…".into(),
        };
        let split = pair_edit_rows(&[row], &language);
        let unified = unified_rows(&split);
        return PreparedToolChange {
            kind: PreparedKind::Edit {
                split: Arc::new(split),
                unified: Arc::new(unified),
            },
            additions: 0,
            deletions: 0,
            omitted: 0,
        };
    };

    let mut additions = 0;
    let mut deletions = 0;
    let mut total = 0_usize;
    let mut preview = Vec::with_capacity(MAX_LINES);
    for line in diff.lines() {
        total += 1;
        match line.as_bytes().first() {
            Some(b'+') => additions += 1,
            Some(b'-') => deletions += 1,
            _ => {}
        }
        if preview.len() < MAX_LINES {
            preview.push(parse_display_line(line, format));
        }
    }
    let split = pair_edit_rows(&preview, &language);
    let unified = unified_rows(&split);
    PreparedToolChange {
        kind: PreparedKind::Edit {
            split: Arc::new(split),
            unified: Arc::new(unified),
        },
        additions,
        deletions,
        omitted: total.saturating_sub(preview.len()),
    }
}

pub(crate) fn prepare_write(path: &str, content: &str) -> PreparedToolChange {
    let _timing = crate::performance::OperationTiming::new(
        crate::performance::OperationKind::ToolPreview,
        content.len(),
    );
    let additions = content.lines().count();
    let mut rows = content
        .lines()
        .take(MAX_LINES)
        .enumerate()
        .map(|(index, line)| SideLine {
            kind: ChangeKind::Addition,
            number: u64::try_from(index)
                .ok()
                .and_then(|value| value.checked_add(1)),
            syntax: HighlightedText::plain(replace_tabs(line)),
        })
        .collect::<Vec<_>>();
    highlight_side_lines(rows.iter_mut(), &language_for_path(path));
    PreparedToolChange {
        omitted: additions.saturating_sub(rows.len()),
        kind: PreparedKind::Write(Arc::new(rows)),
        additions,
        deletions: 0,
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
                    SideLine {
                        kind: row.kind,
                        number: Some(number),
                        syntax: HighlightedText::plain(replace_tabs(&row.content)),
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
                    syntax: HighlightedText::plain(replace_tabs(&row.content)),
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
        paired.iter_mut().filter_map(|row| row.old.as_mut()),
        language,
    );
    highlight_side_lines(
        paired.iter_mut().filter_map(|row| row.new.as_mut()),
        language,
    );
    paired
}

fn unified_rows(rows: &[PairedLine]) -> Vec<SideLine> {
    rows.iter()
        .flat_map(|row| match (&row.old, &row.new) {
            (Some(old), Some(new))
                if old.kind == new.kind
                    && matches!(old.kind, ChangeKind::Context | ChangeKind::Ellipsis) =>
            {
                [Some(old), None]
            }
            (old, new) => [old.as_ref(), new.as_ref()],
        })
        .flatten()
        .cloned()
        .collect()
}

fn numbered_side(row: &ChangeLine, next: &mut u64) -> SideLine {
    let number = row.number.unwrap_or(*next);
    *next = number.saturating_add(1);
    SideLine {
        kind: row.kind,
        number: Some(number),
        syntax: HighlightedText::plain(replace_tabs(&row.content)),
    }
}

fn highlight_side_lines<'a>(lines: impl Iterator<Item = &'a mut SideLine>, language: &str) {
    let lines = lines.collect::<Vec<_>>();
    if lines.is_empty() {
        return;
    }
    let source = lines
        .iter()
        .map(|line| line.syntax.shared_text())
        .collect::<Vec<_>>();
    let source = source.iter().map(AsRef::as_ref).collect::<Vec<_>>();
    for (line, syntax) in lines.into_iter().zip(highlight_lines(&source, language)) {
        line.syntax = syntax;
    }
}

fn flush_changes(
    paired: &mut Vec<PairedLine>,
    deletions: &mut Vec<SideLine>,
    additions: &mut Vec<SideLine>,
) {
    for index in 0..deletions.len().max(additions.len()) {
        paired.push(PairedLine {
            old: deletions.get(index).cloned(),
            new: additions.get(index).cloned(),
        });
    }
    deletions.clear();
    additions.clear();
}

fn apply_line_delta(number: u64, delta: i64) -> u64 {
    if delta < 0 {
        number.saturating_sub(delta.unsigned_abs())
    } else {
        number.saturating_add(delta.unsigned_abs())
    }
}

fn replace_tabs(content: &str) -> String {
    content.replace('\t', "   ")
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
    fn writes_are_highlighted_and_bounded_before_rendering() {
        let content = (0..=MAX_LINES)
            .map(|index| format!("let value_{index} = {index};"))
            .collect::<Vec<_>>()
            .join("\n");
        let prepared = prepare_write("src/main.rs", &content);
        let rows = prepared.unified_rows();

        assert_eq!(rows.len(), MAX_LINES);
        assert_eq!(prepared.omitted(), 1);
        assert!(rows.iter().all(|row| row.syntax.has_highlights()));
    }

    #[test]
    fn numbered_edits_pair_replacements_and_keep_independent_numbers() {
        let prepared = prepare_edit(
            "src/main.rs",
            Some(" 10 context\n- 11 old one\n- 12 old two\n+ 11 new one\n 13 tail"),
            EditDiffFormat::Numbered,
        );
        let rows = prepared.split_rows().expect("expected edit rows");

        assert_eq!(rows[0].old.as_ref().and_then(|line| line.number), Some(10));
        assert_eq!(rows[0].new.as_ref().and_then(|line| line.number), Some(10));
        assert_eq!(rows[1].old.as_ref().and_then(|line| line.number), Some(11));
        assert_eq!(rows[1].new.as_ref().and_then(|line| line.number), Some(11));
        assert_eq!(rows[2].old.as_ref().and_then(|line| line.number), Some(12));
        assert!(rows[2].new.is_none());
        assert_eq!(rows[3].new.as_ref().and_then(|line| line.number), Some(12));
    }

    #[test]
    fn argument_preview_numbers_remain_code_not_gutter_metadata() {
        let row = parse_display_line("- 123 source", EditDiffFormat::Unnumbered);

        assert_eq!(row.kind, ChangeKind::Deletion);
        assert_eq!(row.number, None);
        assert_eq!(row.content, "123 source");
    }
}
