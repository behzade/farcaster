//! Renderer-neutral diff planning.
//!
//! The patch parsing, split-row alignment, and intraline-span behavior are
//! adapted from `@pierre/diffs` at Pierre commit
//! `55a941914056af44c78c4ba607b37130f189fb70`. The implementation was rewritten
//! in Rust to produce immutable GPUI-neutral rows rather than HAST or DOM nodes.

mod format;
mod parser;
mod plan_builder;
mod word_diff;

#[cfg(test)]
mod tests;

use std::{fmt, ops::Range};

/// Presentation layout requested by the consumer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffLayout {
    /// Deletions followed by additions.
    Unified,
    /// Old and new lines paired into two columns.
    Split,
}

/// Controls bounded planning and optional intraline changed ranges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiffPlanOptions {
    pub layout: DiffLayout,
    /// Maximum rows retained per file. Parsing and statistics remain complete.
    pub max_rows_per_file: Option<usize>,
    pub intraline_changes: bool,
    /// Lines above this byte length receive one whole-line changed range.
    pub max_intraline_bytes: usize,
}

impl DiffPlanOptions {
    pub const fn new(layout: DiffLayout) -> Self {
        Self {
            layout,
            max_rows_per_file: None,
            intraline_changes: true,
            max_intraline_bytes: 2_000,
        }
    }

    pub const fn bounded(layout: DiffLayout, max_rows_per_file: usize) -> Self {
        Self {
            layout,
            max_rows_per_file: Some(max_rows_per_file),
            intraline_changes: true,
            max_intraline_bytes: 2_000,
        }
    }
}

/// A complete, immutable plan for every file in one patch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffRenderPlan {
    pub files: Vec<FileDiffPlan>,
    pub source_bytes: usize,
}

impl DiffRenderPlan {
    pub fn retained_rows(&self) -> usize {
        self.files.iter().map(|file| file.rows.len()).sum()
    }

    pub fn total_rows(&self) -> usize {
        self.files.iter().map(FileDiffPlan::total_rows).sum()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileChangeKind {
    Changed,
    Added,
    Deleted,
    Renamed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiffPlan {
    pub old_path: Option<String>,
    pub path: Option<String>,
    pub kind: FileChangeKind,
    pub additions: usize,
    pub deletions: usize,
    pub rows: Vec<DiffPlanRow>,
    pub omitted_rows: usize,
}

impl FileDiffPlan {
    pub fn total_rows(&self) -> usize {
        self.rows.len().saturating_add(self.omitted_rows)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiffPlanRow {
    Hunk { header: String },
    Line(DiffPlanLine),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffLineKind {
    Context,
    Change,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffPlanLine {
    pub kind: DiffLineKind,
    pub old: Option<DiffPlanCell>,
    pub new: Option<DiffPlanCell>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffPlanCell {
    pub line_number: u64,
    pub text: String,
    /// UTF-8 byte ranges receiving the stronger intraline-change background.
    pub changed: Vec<Range<usize>>,
    pub no_newline: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffPlanErrorKind {
    EmptyPatch,
    MissingFileHeader,
    InvalidHunkHeader,
    MissingHunk,
    InvalidHunkLine,
    HunkLineCountMismatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffPlanError {
    pub kind: DiffPlanErrorKind,
    pub line: usize,
    pub message: String,
}

impl DiffPlanError {
    pub(crate) fn new(kind: DiffPlanErrorKind, line: usize, message: impl Into<String>) -> Self {
        Self {
            kind,
            line,
            message: message.into(),
        }
    }
}

impl fmt::Display for DiffPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} at patch line {}", self.message, self.line)
    }
}

impl std::error::Error for DiffPlanError {}

/// Parse a standard git or unified patch into rows ready for a native renderer.
///
/// The result contains strings, semantic row kinds, line numbers, and changed
/// byte ranges only. Font shaping, colors, clipping, and glyph painting remain
/// the renderer's responsibility.
pub fn plan_patch(patch: &str, options: DiffPlanOptions) -> Result<DiffRenderPlan, DiffPlanError> {
    parser::plan_patch(patch, options)
}
