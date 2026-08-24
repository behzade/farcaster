//! Focus-trapped view of file operations retained in Pi session records.

use gpui::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    ScrollHandle, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};

use super::super::{
    PiApp,
    changes::{DiffSurface, FullDiffMode, RepositoryDiffState},
};
use crate::{
    assets::AppIcon,
    diff_element::{DiffCell, DiffElement, DiffPaintRow, DiffTone},
    primitives::{ButtonTone, icon_button, section_heading},
    repository::{ChangeKind, ChangeLayer},
    session_changes::FileChangeKind,
    syntax_highlight::{DiffLineKind, HighlightedDiff, HighlightedDiffLine},
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
        let header = diff_header(surface);
        let path = header.display_path.clone();
        let accessible_path = header.accessible_path.clone();
        let close = entity.clone();
        let open = entity.clone();
        let open_path = header.path.clone();
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
                            .child(section_heading(header.title))
                            .child(
                                div()
                                    .id("diff-file-path")
                                    .aria_label(accessible_path)
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
                    .child(icon_button(
                        "close-full-diff",
                        AppIcon::X,
                        "Close",
                        ButtonTone::Quiet,
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
                            .child(
                                div()
                                    .text_color(THEME.colors.muted)
                                    .child(header.state),
                            )
                            .child(
                                div()
                                    .font_family(MONO_FONT_FAMILY)
                                    .text_color(THEME.colors.success)
                                    .child(header.additions),
                            )
                            .child(
                                div()
                                    .font_family(MONO_FONT_FAMILY)
                                    .text_color(THEME.colors.error)
                                    .child(header.deletions),
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
                            .when(header.exists, |controls| {
                                controls.child(icon_button(
                                    "open-diff-file",
                                    AppIcon::ArrowSquareOut,
                                    "Open in Neovim",
                                    ButtonTone::Quiet,
                                    move |window, cx| {
                                        let _ = open.update(cx, |this, cx| {
                                            this.close_file_diff(window, cx);
                                            this.open_file_editor(open_path.clone(), window, cx);
                                        });
                                    },
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
                        self.changes.diff_syntax.as_deref(),
                        mode,
                        &self.changes.diff_scroll,
                    ))
                    .into_any_element(),
                DiffSurface::Preview(_, _diff, reason) => div()
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
                        self.changes.diff_syntax.as_deref(),
                        mode,
                        &self.changes.diff_scroll,
                    ))
                    .into_any_element(),
                DiffSurface::Repository {
                    state: RepositoryDiffState::Loading,
                    ..
                } => div()
                    .id("repository-diff-loading")
                    .role(Role::Status)
                    .flex_1()
                    .p(THEME.space.md)
                    .text_color(THEME.colors.muted)
                    .child("Loading repository diff…")
                    .into_any_element(),
                DiffSurface::Repository {
                    state: RepositoryDiffState::Ready(diff),
                    ..
                } if diff.patch.is_empty() => div()
                    .id("repository-diff-empty")
                    .role(Role::Status)
                    .flex_1()
                    .p(THEME.space.md)
                    .text_color(THEME.colors.subtle)
                    .child("This change has no textual diff.")
                    .into_any_element(),
                DiffSurface::Repository {
                    state: RepositoryDiffState::Ready(_),
                    ..
                } => render_patch(
                    self.changes.diff_syntax.as_deref(),
                    mode,
                    &self.changes.diff_scroll,
                ),
                DiffSurface::Repository {
                    state: RepositoryDiffState::Error(error),
                    ..
                } => div()
                    .id("repository-diff-error")
                    .role(Role::Alert)
                    .flex_1()
                    .p(THEME.space.md)
                    .text_color(THEME.colors.error)
                    .child(error.clone())
                    .into_any_element(),
            })
            .into_any_element()
    }
}

struct DiffHeader {
    path: std::path::PathBuf,
    display_path: String,
    accessible_path: String,
    title: &'static str,
    state: String,
    additions: String,
    deletions: String,
    exists: bool,
}

fn diff_header(surface: &DiffSurface) -> DiffHeader {
    match surface {
        DiffSurface::Ready(file, _) | DiffSurface::Preview(file, _, _) => session_diff_header(
            file,
            match surface {
                DiffSurface::Preview(_, _, _) => "File change preview",
                _ => "File changes",
            },
        ),
        DiffSurface::Error(file, _) => session_diff_header(file, "Changes unavailable"),
        DiffSurface::Repository { target, state } => {
            let (additions, deletions, exists) = match state {
                RepositoryDiffState::Ready(diff) => (diff.additions, diff.deletions, diff.exists),
                RepositoryDiffState::Loading | RepositoryDiffState::Error(_) => {
                    (None, None, target.exists)
                }
            };
            let path = target.absolute_path();
            let target_path = visible_diff_path(&path);
            let (display_path, accessible_path) =
                target.original_relative_path.as_ref().map_or_else(
                    || (target_path.clone(), target_path.clone()),
                    |original| {
                        let original = visible_diff_path(&target.workspace_root.join(original));
                        let operation = if target.kind == ChangeKind::Copied {
                            "Copied"
                        } else {
                            "Renamed"
                        };
                        (
                            format!("{original} -> {target_path}"),
                            format!("{operation} from {original} to {target_path}"),
                        )
                    },
                );
            DiffHeader {
                path,
                display_path,
                accessible_path,
                title: repository_diff_title(target.layer),
                state: repository_change_label(&target.kind).to_owned(),
                additions: count_label('+', additions),
                deletions: count_label('-', deletions),
                exists,
            }
        }
    }
}

fn session_diff_header(
    file: &crate::session_changes::FileChange,
    title: &'static str,
) -> DiffHeader {
    let display_path = visible_diff_path(&file.path);
    DiffHeader {
        path: file.path.clone(),
        display_path: display_path.clone(),
        accessible_path: display_path,
        title,
        state: match file.kind {
            FileChangeKind::Edited => "Edited",
            FileChangeKind::Written => "Written",
            FileChangeKind::Mixed => "Edited and written",
        }
        .to_owned(),
        additions: count_label('+', file.additions),
        deletions: count_label('-', file.deletions),
        exists: file.exists,
    }
}

fn visible_diff_path(path: &std::path::Path) -> String {
    path.to_string_lossy()
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn count_label(prefix: char, count: Option<u64>) -> String {
    count.map_or_else(|| format!("{prefix}—"), |count| format!("{prefix}{count}"))
}

const fn repository_diff_title(layer: ChangeLayer) -> &'static str {
    match layer {
        ChangeLayer::GitIndex => "Git staged changes",
        ChangeLayer::GitWorkingTree => "Git working-tree changes",
        ChangeLayer::GitConflict => "Git conflict",
        ChangeLayer::GitUntracked => "Git untracked file",
        ChangeLayer::JujutsuWorkingCopy => "Jujutsu current change",
    }
}

const fn repository_change_label(kind: &ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "Added",
        ChangeKind::Modified => "Modified",
        ChangeKind::Deleted => "Deleted",
        ChangeKind::Renamed => "Renamed",
        ChangeKind::Copied => "Copied",
        ChangeKind::TypeChanged => "Type changed",
        ChangeKind::Untracked => "Untracked",
        ChangeKind::Conflict => "Conflict",
        ChangeKind::Unknown(_) => "Changed",
    }
}

fn render_patch(
    syntax: Option<&HighlightedDiff>,
    mode: FullDiffMode,
    scroll: &ScrollHandle,
) -> AnyElement {
    let Some(syntax) = syntax else {
        return div().child("Preparing diff…").into_any_element();
    };
    let accessible = accessible_diff_text(syntax);
    let diff = match (mode, syntax) {
        (FullDiffMode::Unified, HighlightedDiff::Unified(lines)) => {
            let rows = lines.clone();
            DiffElement::unified(
                rows.len(),
                THEME.type_scale.line_body,
                px(0.0),
                move |index| rows.get(index).map(full_diff_cell),
            )
        }
        (FullDiffMode::Split, HighlightedDiff::Split { old, new }) => {
            let old = old.clone();
            let new = new.clone();
            let count = old.len().max(new.len());
            DiffElement::split(count, THEME.type_scale.line_body, px(0.0), move |index| {
                DiffPaintRow {
                    old: old.get(index).map(full_diff_cell),
                    new: new.get(index).map(full_diff_cell),
                }
            })
        }
        _ => return div().child("Preparing diff…").into_any_element(),
    };
    div()
        .id("full-diff-scroll")
        .role(Role::Document)
        .aria_label("File diff")
        .a11y_synthetic_children(move |builder| {
            builder.parent_node().set_value(accessible);
        })
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .bg(THEME.colors.canvas)
        .child(diff)
        .into_any_element()
}

fn accessible_diff_text(diff: &HighlightedDiff) -> String {
    const LIMIT: usize = 64 * 1024;
    let mut accessible = String::new();
    let mut append = |label: &str, line: &HighlightedDiffLine| {
        if accessible.len() >= LIMIT {
            return;
        }
        accessible.push_str(label);
        accessible.push_str(&line.text.shared_text());
        accessible.push('\n');
        if accessible.len() > LIMIT {
            let mut end = LIMIT;
            while !accessible.is_char_boundary(end) {
                end = end.saturating_sub(1);
            }
            accessible.truncate(end);
        }
    };
    match diff {
        HighlightedDiff::Unified(lines) => {
            for line in lines.iter() {
                append("", line);
            }
        }
        HighlightedDiff::Split { old, new } => {
            for index in 0..old.len().max(new.len()) {
                match (old.get(index), new.get(index)) {
                    (Some(old), Some(new)) if old.text.shared_text() == new.text.shared_text() => {
                        append("", old);
                    }
                    (Some(old), Some(new)) => {
                        append("Old: ", old);
                        append("New: ", new);
                    }
                    (Some(old), None) => append("Old: ", old),
                    (None, Some(new)) => append("New: ", new),
                    (None, None) => {}
                }
            }
        }
    }
    accessible
}

fn full_diff_cell(line: &HighlightedDiffLine) -> DiffCell {
    DiffCell {
        gutter: None,
        text: line.text.clone(),
        tone: match line.kind {
            DiffLineKind::Context => DiffTone::Context,
            DiffLineKind::Addition => DiffTone::Addition,
            DiffLineKind::Deletion => DiffTone::Deletion,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn painted_diff_retains_bounded_accessible_text() {
        let patch = format!(
            "--- a/file.txt\n+++ b/file.txt\n@@ -1 +1 @@\n-old\n+{}\n",
            "🙂".repeat(40_000)
        );
        let diff = HighlightedDiff::new(
            "file.txt",
            &patch,
            crate::syntax_highlight::DiffHighlightMode::Unified,
        );
        let accessible = accessible_diff_text(&diff);

        assert!(accessible.contains("-old"));
        assert!(accessible.len() <= 64 * 1024);
        assert!(accessible.is_char_boundary(accessible.len()));
    }
}
