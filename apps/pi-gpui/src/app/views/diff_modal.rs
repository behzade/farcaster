//! Focus-trapped, on-demand complete diff surface.

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};
use gpui_component::text::TextView;

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
    pub(super) fn render_diff_modal(&self, entity: WeakEntity<Self>) -> AnyElement {
        let Some(surface) = self.changes.diff.as_ref() else {
            return div().into_any_element();
        };
        let (file, title) = match surface {
            DiffSurface::Loading(file) => (file, "Loading complete diff…"),
            DiffSurface::Ready(file, _) => (file, "Complete diff"),
            DiffSurface::Preview(file, _, _) => (file, "Tool diff preview"),
            DiffSurface::Error(file, _) => (file, "Diff unavailable"),
        };
        let path = file.path.to_string_lossy().into_owned();
        let state = match file.kind {
            FileChangeKind::Modified => "Modified",
            FileChangeKind::Added => "Added",
            FileChangeKind::Deleted => "Deleted",
            FileChangeKind::Renamed => "Renamed",
            FileChangeKind::Binary => "Binary",
            FileChangeKind::Unavailable => "Unavailable",
        };
        let counts = match (file.additions, file.deletions) {
            (Some(additions), Some(deletions)) => format!("{state} · +{additions} -{deletions}"),
            _ => state.into(),
        };
        let close = entity.clone();
        let split = entity.clone();
        let unified = entity.clone();
        let open_path = file.path.clone();
        div()
            .w_full()
            .h_full()
            .p(THEME.space.md)
            .flex()
            .flex_col()
            .gap(THEME.space.sm)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(THEME.space.sm)
                    .child(div().min_w_0().flex_1().child(section_heading(title)))
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
                    .items_center()
                    .justify_between()
                    .gap(THEME.space.sm)
                    .child(
                        div()
                            .min_w_0()
                            .font_family(MONO_FONT_FAMILY)
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.muted)
                            .child(path),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_family(MONO_FONT_FAMILY)
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(counts),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(THEME.space.xs)
                    .child(button(
                        "diff-mode-split",
                        "Split",
                        if self.changes.diff_mode == FullDiffMode::Split {
                            ButtonTone::Accent
                        } else {
                            ButtonTone::Quiet
                        },
                        true,
                        move |_, cx| {
                            let _ = split.update(cx, |this, cx| {
                                this.changes.diff_mode = FullDiffMode::Split;
                                cx.notify();
                            });
                        },
                    ))
                    .child(button(
                        "diff-mode-unified",
                        "Unified",
                        if self.changes.diff_mode == FullDiffMode::Unified {
                            ButtonTone::Accent
                        } else {
                            ButtonTone::Quiet
                        },
                        true,
                        move |_, cx| {
                            let _ = unified.update(cx, |this, cx| {
                                this.changes.diff_mode = FullDiffMode::Unified;
                                cx.notify();
                            });
                        },
                    ))
                    .when(file.exists, |controls| {
                        controls.child(button(
                            "open-diff-file",
                            "Open file",
                            ButtonTone::Quiet,
                            true,
                            move |_, cx| cx.open_with_system(&open_path),
                        ))
                    }),
            )
            .child(match surface {
                DiffSurface::Loading(_) => div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(THEME.colors.muted)
                    .child("Loading…")
                    .into_any_element(),
                DiffSurface::Error(_, error) => div()
                    .flex_1()
                    .p(THEME.space.md)
                    .text_color(THEME.colors.error)
                    .child(error.clone())
                    .into_any_element(),
                DiffSurface::Ready(_, diff) if diff.binary => div()
                    .flex_1()
                    .p(THEME.space.md)
                    .font_family(MONO_FONT_FAMILY)
                    .text_color(THEME.colors.muted)
                    .child("Binary content differs from HEAD; no textual diff is available.")
                    .into_any_element(),
                DiffSurface::Ready(_, diff) => render_patch(
                    &diff.patch,
                    self.changes.diff_mode,
                    &self.changes.diff_scroll,
                ),
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
                            .child("Current full HEAD diff is unavailable. Showing the retained tool preview; it may be incomplete."),
                    )
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(reason.clone()),
                    )
                    .child(render_patch(
                        &diff.patch,
                        self.changes.diff_mode,
                        &self.changes.diff_scroll,
                    ))
                    .into_any_element(),
            })
            .into_any_element()
    }
}

fn render_patch(patch: &str, mode: FullDiffMode, scroll: &gpui::ScrollHandle) -> AnyElement {
    match mode {
        FullDiffMode::Unified => div()
            .id("full-unified-diff")
            .flex_1()
            .min_h_0()
            .overflow_scroll()
            .track_scroll(scroll)
            .child(diff_text("full-unified-text", patch))
            .into_any_element(),
        FullDiffMode::Split => {
            let (old, new) = split_patch(patch);
            div()
                .id("full-split-diff")
                .flex_1()
                .min_h_0()
                .flex()
                .overflow_scroll()
                .track_scroll(scroll)
                .child(
                    div()
                        .w_1_2()
                        .min_w(px(480.0))
                        .border_r(THEME.border)
                        .border_color(THEME.colors.border)
                        .child(diff_text("full-split-old", &old)),
                )
                .child(
                    div()
                        .w_1_2()
                        .min_w(px(480.0))
                        .child(diff_text("full-split-new", &new)),
                )
                .into_any_element()
        }
    }
}

fn diff_text(id: &'static str, text: &str) -> TextView {
    let fence = if text.contains("````") {
        "`````"
    } else {
        "````"
    };
    TextView::markdown(id, format!("{fence}diff\n{text}\n{fence}"))
        .selectable(true)
        .whitespace_nowrap()
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small)
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
