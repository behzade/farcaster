//! Cached tree-sitter highlighting rendered through GPUI text runs.

use std::{ops::Range, sync::Arc};

use gpui::{HighlightStyle, SharedString};
use gpui_component::highlighter::{HighlightTheme, SyntaxHighlighter};
use ropey::Rope;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HighlightedText {
    text: SharedString,
    highlights: Arc<Vec<(Range<usize>, HighlightStyle)>>,
}

impl HighlightedText {
    pub(crate) fn plain(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            highlights: Arc::default(),
        }
    }

    pub(crate) fn shared_text(&self) -> SharedString {
        self.text.clone()
    }

    pub(crate) fn runs(&self, default_style: &gpui::TextStyle) -> Vec<gpui::TextRun> {
        let mut runs = Vec::new();
        let mut offset = 0;
        for (range, highlight) in self.highlights.iter() {
            debug_assert!(self.text.is_char_boundary(range.start));
            debug_assert!(self.text.is_char_boundary(range.end));
            debug_assert!(offset <= range.start);
            if offset < range.start {
                runs.push(default_style.to_run(range.start - offset));
            }
            runs.push(
                default_style
                    .clone()
                    .highlight(*highlight)
                    .to_run(range.len()),
            );
            offset = range.end;
        }
        if offset < self.text.len() {
            runs.push(default_style.to_run(self.text.len() - offset));
        }
        runs
    }

    fn into_lines(self) -> Vec<Self> {
        let mut lines = Vec::new();
        let mut offset = 0;
        let mut highlight_index = 0;
        for line in self.text.split_inclusive('\n') {
            let text = line.strip_suffix('\n').unwrap_or(line);
            let end = offset + text.len();
            while self
                .highlights
                .get(highlight_index)
                .is_some_and(|(range, _)| range.end <= offset)
            {
                highlight_index += 1;
            }
            let mut line_highlights = Vec::new();
            for (range, style) in self.highlights[highlight_index..]
                .iter()
                .take_while(|(range, _)| range.start < end)
            {
                let start = range.start.max(offset);
                let finish = range.end.min(end);
                if start < finish {
                    line_highlights.push((start - offset..finish - offset, *style));
                }
            }
            lines.push(Self {
                text: text.to_owned().into(),
                highlights: Arc::new(line_highlights),
            });
            offset += line.len();
        }
        lines
    }

    fn into_diff_lines(self) -> Arc<Vec<HighlightedDiffLine>> {
        let mut in_hunk = false;
        Arc::new(
            self.into_lines()
                .into_iter()
                .map(|text| {
                    if text.text.starts_with("diff --git ") {
                        in_hunk = false;
                    } else if text.text.starts_with("@@") {
                        in_hunk = true;
                    }
                    let kind = if text.text.starts_with('-')
                        && (in_hunk || !is_git_old_header(&text.text))
                    {
                        DiffLineKind::Deletion
                    } else if text.text.starts_with('+')
                        && (in_hunk || !is_git_new_header(&text.text))
                    {
                        DiffLineKind::Addition
                    } else {
                        DiffLineKind::Context
                    };
                    HighlightedDiffLine { kind, text }
                })
                .collect(),
        )
    }

    #[cfg(test)]
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    #[cfg(test)]
    pub(crate) fn has_highlights(&self) -> bool {
        !self.highlights.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HighlightedDiffLine {
    pub kind: DiffLineKind,
    pub text: HighlightedText,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiffLineKind {
    Context,
    Addition,
    Deletion,
}

pub(crate) fn highlight_lines(lines: &[&str], language: &str) -> Vec<HighlightedText> {
    let mut source = lines.join("\n");
    source.push('\n');
    highlight(source, language).into_lines()
}

pub(crate) fn highlight(text: String, language: &str) -> HighlightedText {
    crate::performance::count_highlight_bytes(text.len());
    if text.is_empty() || language == "text" {
        return HighlightedText::plain(text);
    }
    let rope = Rope::from_str(&text);
    let mut highlighter = SyntaxHighlighter::new(language);
    highlighter.update(None, &rope, None);
    let highlights = highlighter.styles(&(0..text.len()), HighlightTheme::default_dark().as_ref());
    HighlightedText {
        text: text.into(),
        highlights: Arc::new(highlights),
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum DiffHighlightMode {
    Unified,
    Split,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HighlightedDiff {
    Unified(Arc<Vec<HighlightedDiffLine>>),
    Split {
        old: Arc<Vec<HighlightedDiffLine>>,
        new: Arc<Vec<HighlightedDiffLine>>,
    },
}

impl HighlightedDiff {
    pub(crate) fn new(path: &str, patch: &str, mode: DiffHighlightMode) -> Self {
        let language = language_for_path(path);
        match mode {
            DiffHighlightMode::Unified => {
                Self::Unified(highlight(patch.to_owned(), &language).into_diff_lines())
            }
            DiffHighlightMode::Split => {
                let (old, new) = split_patch(patch);
                Self::Split {
                    old: highlight(old, &language).into_diff_lines(),
                    new: highlight(new, &language).into_diff_lines(),
                }
            }
        }
    }

    pub(crate) fn mode(&self) -> DiffHighlightMode {
        match self {
            Self::Unified(_) => DiffHighlightMode::Unified,
            Self::Split { .. } => DiffHighlightMode::Split,
        }
    }
}

pub(crate) fn language_for_path(path: &str) -> String {
    let path = std::path::Path::new(path);
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

fn split_patch(patch: &str) -> (String, String) {
    let mut old = String::new();
    let mut new = String::new();
    let mut deletions = Vec::new();
    let mut additions = Vec::new();
    let mut in_hunk = false;
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            in_hunk = false;
        } else if line.starts_with("@@") {
            in_hunk = true;
        }
        if line.starts_with('-') && (in_hunk || !is_git_old_header(line)) {
            deletions.push(line);
        } else if line.starts_with('+') && (in_hunk || !is_git_new_header(line)) {
            additions.push(line);
        } else {
            flush_split_block(&mut old, &mut new, &mut deletions, &mut additions);
            old.push_str(line);
            old.push('\n');
            new.push_str(line);
            new.push('\n');
        }
    }
    flush_split_block(&mut old, &mut new, &mut deletions, &mut additions);
    (old, new)
}

fn flush_split_block(
    old: &mut String,
    new: &mut String,
    deletions: &mut Vec<&str>,
    additions: &mut Vec<&str>,
) {
    for index in 0..deletions.len().max(additions.len()) {
        if let Some(line) = deletions.get(index) {
            old.push_str(line);
        }
        old.push('\n');
        if let Some(line) = additions.get(index) {
            new.push_str(line);
        }
        new.push('\n');
    }
    deletions.clear();
    additions.clear();
}

fn is_git_old_header(line: &str) -> bool {
    line.starts_with("--- a/") || line == "--- /dev/null"
}

fn is_git_new_header(line: &str) -> bool {
    line.starts_with("+++ b/") || line == "+++ /dev/null"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_paths_select_the_syntax_language() {
        assert_eq!(language_for_path("src/main.ts"), "ts");
        assert_eq!(language_for_path("Dockerfile"), "dockerfile");
        assert_eq!(language_for_path("LICENSE"), "text");
    }

    #[test]
    fn plain_text_keeps_source_without_highlight_work() {
        let text = highlight("hello".into(), "text");
        assert_eq!(text.text(), "hello");
        assert!(text.highlights.is_empty());
    }

    #[test]
    fn source_is_highlighted_once_into_cached_ranges() {
        let text = highlight("fn main() { let answer = 42; }".into(), "rs");
        assert_eq!(text.text(), "fn main() { let answer = 42; }");
        assert!(!text.highlights.is_empty());
    }

    #[test]
    fn direct_paint_runs_cover_the_complete_source() {
        let text = highlight("fn main() {}".into(), "rs");
        let runs = text.runs(&gpui::TextStyle::default());

        assert_eq!(
            runs.iter().map(|run| run.len).sum::<usize>(),
            text.text.len()
        );
        assert!(runs.len() > 1);
    }

    #[test]
    fn batched_highlighting_preserves_lines_including_empty_ones() {
        let lines = highlight_lines(&["fn main() {", "", "}"], "rs");

        assert_eq!(
            lines.iter().map(HighlightedText::text).collect::<Vec<_>>(),
            ["fn main() {", "", "}"]
        );
        assert!(lines[0].has_highlights());
    }

    #[test]
    fn diff_lines_classify_changes_without_coloring_file_headers() {
        let text = HighlightedText::plain(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n",
        );
        let lines = text.into_diff_lines();
        assert_eq!(
            lines.iter().map(|line| line.kind).collect::<Vec<_>>(),
            [
                DiffLineKind::Context,
                DiffLineKind::Context,
                DiffLineKind::Context,
                DiffLineKind::Context,
                DiffLineKind::Deletion,
                DiffLineKind::Addition,
            ]
        );
    }
}
