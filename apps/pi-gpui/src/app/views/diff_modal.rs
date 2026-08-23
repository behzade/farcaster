//! Focus-trapped view of file operations retained in Pi session records.

use gpui::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};

use super::super::{
    PiApp,
    changes::{DiffSurface, FullDiffMode},
};
use crate::{
    assets::AppIcon,
    diff_element::{DiffCell, DiffElement, DiffPaintRow, DiffTone},
    primitives::{ButtonTone, icon_button, section_heading},
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
        let open = entity.clone();
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
            })
            .into_any_element()
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
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .bg(THEME.colors.canvas)
        .child(diff)
        .into_any_element()
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
