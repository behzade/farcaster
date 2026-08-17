//! Focus-trapped view of file operations retained in Pi session records.

use std::path::Path;

use gpui::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};

use super::super::{
    PiApp,
    changes::{DiffSurface, FullDiffMode},
};
use crate::{
    primitives::{ButtonTone, button, section_heading},
    session_changes::FileChangeKind,
    theme::{MONO_FONT_FAMILY, THEME},
};

impl PiApp {
    pub(super) fn render_diff_modal(
        &self,
        entity: WeakEntity<Self>,
        mode: FullDiffMode,
    ) -> AnyElement {
        let Some(surface) = self.changes.diff.as_ref() else {
            return div().into_any_element();
        };
        let (file, title) = match surface {
            DiffSurface::Ready(file, _) => (file, "File changes"),
            DiffSurface::Preview(file, _, _) => (file, "File change preview"),
            DiffSurface::Error(file, _) => (file, "Changes unavailable"),
        };
        let path = file.path.to_string_lossy().into_owned();
        let state = match file.kind {
            FileChangeKind::Edited => "Edited",
            FileChangeKind::Written => "Written",
            FileChangeKind::Mixed => "Edited and written",
        };
        let additions = file
            .additions
            .map_or_else(|| "+—".into(), |count| format!("+{count}"));
        let deletions = file
            .deletions
            .map_or_else(|| "-—".into(), |count| format!("-{count}"));
        let close = entity.clone();
        let open_path = file.path.clone();
        div()
            .w_full()
            .h_full()
            .min_h_0()
            .p(THEME.space.md)
            .flex()
            .flex_col()
            .gap(THEME.space.sm)
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap(THEME.space.md)
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(section_heading(title))
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .font_family(MONO_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(THEME.colors.text)
                                    .child(path),
                            ),
                    )
                    .child(button(
                        "close-full-diff",
                        "Close",
                        ButtonTone::Neutral,
                        true,
                        move |window, cx| {
                            let _ = close.update(cx, |this, cx| this.close_file_diff(window, cx));
                        },
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(THEME.space.md)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(THEME.space.sm)
                            .text_size(THEME.type_scale.caption)
                            .child(div().text_color(THEME.colors.muted).child(state))
                            .child(
                                div()
                                    .font_family(MONO_FONT_FAMILY)
                                    .text_color(THEME.colors.success)
                                    .child(additions),
                            )
                            .child(
                                div()
                                    .font_family(MONO_FONT_FAMILY)
                                    .text_color(THEME.colors.error)
                                    .child(deletions),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(THEME.space.sm)
                            .child(
                                div()
                                    .text_size(THEME.type_scale.caption)
                                    .text_color(THEME.colors.muted)
                                    .child(match mode {
                                        FullDiffMode::Split => "Split",
                                        FullDiffMode::Unified => "Unified",
                                    }),
                            )
                            .when(file.exists, |controls| {
                                controls.child(button(
                                    "open-diff-file",
                                    "Open file",
                                    ButtonTone::Quiet,
                                    true,
                                    move |_, cx| cx.open_with_system(&open_path),
                                ))
                            }),
                    ),
            )
            .child(match surface {
                DiffSurface::Error(_, error) => div()
                    .flex_1()
                    .p(THEME.space.md)
                    .text_color(THEME.colors.error)
                    .child(error.clone())
                    .into_any_element(),
                DiffSurface::Ready(_, diff) => div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(THEME.space.xs)
                    .when(diff.partial, |body| {
                        body.child(
                            div()
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.warning)
                                .child("Some edits only retained their call arguments, so this record is partial."),
                        )
                    })
                    .child(render_patch(
                        &file.path,
                        &diff.patch,
                        mode,
                        &self.changes.diff_scroll,
                    ))
                    .into_any_element(),
                DiffSurface::Preview(_, diff, reason) => div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(THEME.space.xs)
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.warning)
                            .child("Showing the retained tool-call preview; it may be incomplete."),
                    )
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(reason.clone()),
                    )
                    .child(render_patch(
                        &file.path,
                        &diff.patch,
                        mode,
                        &self.changes.diff_scroll,
                    ))
                    .into_any_element(),
            })
            .into_any_element()
    }
}

fn render_patch(
    _path: &Path,
    patch: &str,
    mode: FullDiffMode,
    scroll: &ScrollHandle,
) -> AnyElement {
    match mode {
        FullDiffMode::Unified => scrollable_diff(
            "full-unified-diff",
            render_diff_document("full-unified", patch),
            scroll,
        ),
        FullDiffMode::Split => {
            let (old, new) = split_patch(patch);
            scrollable_diff(
                "full-split-diff",
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_start()
                    .child(
                        div()
                            .w_1_2()
                            .min_w_0()
                            .border_r(THEME.border)
                            .border_color(THEME.colors.border)
                            .child(render_diff_document("full-split-old", &old)),
                    )
                    .child(
                        div()
                            .w_1_2()
                            .min_w_0()
                            .child(render_diff_document("full-split-new", &new)),
                    ),
                scroll,
            )
        }
    }
}

fn scrollable_diff(
    id: &'static str,
    content: impl IntoElement,
    scroll: &ScrollHandle,
) -> AnyElement {
    div()
        .id(id)
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_y_scroll()
        .overflow_x_hidden()
        .track_scroll(scroll)
        .bg(THEME.colors.canvas)
        .child(content)
        .into_any_element()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffLineKind {
    Context,
    Addition,
    Deletion,
    Metadata,
}

struct DiffBlock {
    kind: DiffLineKind,
    text: String,
    lines: usize,
}

fn render_diff_document(id: &'static str, text: &str) -> AnyElement {
    div()
        .id(id)
        .w_full()
        .min_w_0()
        .children(diff_blocks(text).into_iter().map(render_diff_block))
        .into_any_element()
}

fn render_diff_block(block: DiffBlock) -> AnyElement {
    let (marker, background, color) = match block.kind {
        DiffLineKind::Context => (" ", THEME.colors.canvas, THEME.colors.text),
        DiffLineKind::Addition => ("+", THEME.colors.diff_added, THEME.colors.success),
        DiffLineKind::Deletion => ("-", THEME.colors.diff_deleted, THEME.colors.error),
        DiffLineKind::Metadata => ("", THEME.colors.surface, THEME.colors.subtle),
    };
    let markers = std::iter::repeat_n(marker, block.lines)
        .collect::<Vec<_>>()
        .join("\n");
    div()
        .w_full()
        .flex()
        .items_start()
        .bg(background)
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small)
        .child(
            div()
                .w(px(32.0))
                .flex_none()
                .px(THEME.space.xs)
                .py(px(2.0))
                .whitespace_nowrap()
                .line_height(THEME.type_scale.line_body)
                .text_align(gpui::TextAlign::Center)
                .text_color(color)
                .child(markers),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .whitespace_normal()
                .px(THEME.space.xs)
                .py(px(2.0))
                .line_height(THEME.type_scale.line_body)
                .text_color(color)
                .child(block.text),
        )
        .into_any_element()
}

fn diff_blocks(text: &str) -> Vec<DiffBlock> {
    let mut blocks = Vec::<DiffBlock>::new();
    let lines = text.lines().collect::<Vec<_>>();
    for (index, line) in lines.iter().copied().enumerate() {
        let paired_file_header = (is_git_old_header(line)
            && lines
                .get(index + 1)
                .is_some_and(|line| is_git_new_header(line)))
            || (is_git_new_header(line)
                && index > 0
                && lines
                    .get(index - 1)
                    .is_some_and(|line| is_git_old_header(line)));
        let (kind, content) = classify_diff_line(line, paired_file_header);
        if let Some(block) = blocks.last_mut()
            && block.kind == kind
        {
            block.text.push('\n');
            block.text.push_str(content);
            block.lines = block.lines.saturating_add(1);
        } else {
            blocks.push(DiffBlock {
                kind,
                text: content.to_owned(),
                lines: 1,
            });
        }
    }
    if blocks.is_empty() {
        blocks.push(DiffBlock {
            kind: DiffLineKind::Metadata,
            text: "No recorded text changes".into(),
            lines: 1,
        });
    }
    blocks
}

fn classify_diff_line(line: &str, paired_file_header: bool) -> (DiffLineKind, &str) {
    if line.starts_with("diff --git ")
        || line.starts_with("index ")
        || line.starts_with("@@")
        || paired_file_header
        || line.starts_with("recorded ")
        || line == "\\ No newline at end of file"
    {
        (DiffLineKind::Metadata, line)
    } else if let Some(content) = line.strip_prefix('+') {
        (DiffLineKind::Addition, content)
    } else if let Some(content) = line.strip_prefix('-') {
        (DiffLineKind::Deletion, content)
    } else {
        (
            DiffLineKind::Context,
            line.strip_prefix(' ').unwrap_or(line),
        )
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
            deletions.push(line.to_owned());
        } else if line.starts_with('+') && (in_hunk || !is_git_new_header(line)) {
            additions.push(line.to_owned());
        } else if line == "\\ No newline at end of file" {
            if additions.is_empty() {
                deletions.push(line.to_owned());
            } else {
                additions.push(line.to_owned());
            }
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

fn is_git_old_header(line: &str) -> bool {
    line.starts_with("--- a/") || line == "--- /dev/null"
}

fn is_git_new_header(line: &str) -> bool {
    line.starts_with("+++ b/") || line == "+++ /dev/null"
}

fn flush_split_block(
    old: &mut String,
    new: &mut String,
    deletions: &mut Vec<String>,
    additions: &mut Vec<String>,
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diff_lines_keep_file_syntax_separate_from_change_semantics() {
        assert_eq!(
            classify_diff_line("+fn added() {}", false),
            (DiffLineKind::Addition, "fn added() {}")
        );
        assert_eq!(
            classify_diff_line("-let removed = true;", false),
            (DiffLineKind::Deletion, "let removed = true;")
        );
        assert_eq!(
            classify_diff_line("@@ -1 +1 @@", false),
            (DiffLineKind::Metadata, "@@ -1 +1 @@")
        );
        assert_eq!(
            classify_diff_line("--- comment", false),
            (DiffLineKind::Deletion, "-- comment")
        );
        assert_eq!(
            classify_diff_line("--- a/file.rs", true),
            (DiffLineKind::Metadata, "--- a/file.rs")
        );
    }

    #[test]
    fn adjacent_change_lines_share_one_highlight_block() {
        let blocks = diff_blocks("+one\n+two\n-context\n same");
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].kind, DiffLineKind::Addition);
        assert_eq!(blocks[0].text, "one\ntwo");
        assert_eq!(blocks[0].lines, 2);
        assert_eq!(blocks[1].kind, DiffLineKind::Deletion);
        assert_eq!(blocks[2].kind, DiffLineKind::Context);
    }

    #[test]
    fn split_document_pairs_replacement_blocks_side_by_side() {
        let (old, new) =
            split_patch("@@\n-old one\n-old two\n\\ No newline at end of file\n+new one\n same\n");
        let old = old.lines().collect::<Vec<_>>();
        let new = new.lines().collect::<Vec<_>>();
        assert_eq!(old[1], "-old one");
        assert_eq!(new[1], "+new one");
        assert_eq!(old[2], "-old two");
        assert_eq!(new[2], "");
        assert_eq!(old[3], "\\ No newline at end of file");
        assert_eq!(new[3], "");
        assert_eq!(old[4], " same");
        assert_eq!(new[4], " same");
    }

    #[test]
    fn repeated_change_markers_are_not_mistaken_for_patch_headers() {
        let (old, new) = split_patch(
            "diff --git a/file b/file\n--- a/file\n+++ b/file\n@@\n---danger\n+++value\n--- a/source-text\n+++ b/source-text\n",
        );
        let old = old.lines().collect::<Vec<_>>();
        let new = new.lines().collect::<Vec<_>>();
        assert_eq!(old[4], "---danger");
        assert_eq!(new[4], "+++value");
        assert_eq!(old[5], "--- a/source-text");
        assert_eq!(new[5], "+++ b/source-text");
        assert!(!new.contains(&"---danger"));
        assert!(!old.contains(&"+++value"));
    }
}
