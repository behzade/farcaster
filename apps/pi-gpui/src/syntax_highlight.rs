//! Cached tree-sitter highlighting rendered through GPUI text runs.

use std::{ops::Range, sync::Arc};

use gpui::{HighlightStyle, SharedString, StyledText};
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

    pub(crate) fn element(&self) -> StyledText {
        StyledText::new(self.text.clone()).with_highlights(self.highlights.iter().cloned())
    }

    #[cfg(test)]
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}

pub(crate) fn highlight(text: String, language: &str) -> HighlightedText {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HighlightedDiff {
    pub(crate) unified: HighlightedText,
    pub(crate) old: HighlightedText,
    pub(crate) new: HighlightedText,
}

impl HighlightedDiff {
    pub(crate) fn new(path: &str, patch: &str) -> Self {
        let language = language_for_path(path);
        let (old, new) = split_patch(patch);
        Self {
            unified: highlight(patch.to_owned(), &language),
            old: highlight(old, &language),
            new: highlight(new, &language),
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
}
