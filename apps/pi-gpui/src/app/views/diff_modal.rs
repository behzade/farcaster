//! Focus-trapped view of file operations retained in Pi session records.

use gpui::{
    AnyElement, FontWeight, IntoElement, ParentElement as _, Styled as _, UniformListScrollHandle,
    WeakEntity, div, prelude::FluentBuilder as _, px, uniform_list,
};

use super::super::{
    PiApp,
    changes::{DiffSurface, FullDiffMode},
};
use crate::{
    assets::AppIcon,
    primitives::{ButtonTone, button, icon_button, section_heading},
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
    scroll: &UniformListScrollHandle,
) -> AnyElement {
    let Some(syntax) = syntax else {
        return div().child("Preparing diff…").into_any_element();
    };
    match (mode, syntax) {
        (FullDiffMode::Unified, HighlightedDiff::Unified(lines)) => {
            let lines = lines.clone();
            let count = lines.len();
            uniform_list("full-unified-diff", count, move |range, _, _| {
                range
                    .filter_map(|index| lines.get(index))
                    .map(render_diff_line)
                    .collect::<Vec<_>>()
            })
            .track_scroll(scroll)
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(THEME.colors.canvas)
            .into_any_element()
        }
        (FullDiffMode::Split, HighlightedDiff::Split { old, new }) => {
            let old = old.clone();
            let new = new.clone();
            let count = old.len().max(new.len());
            uniform_list("full-split-diff", count, move |range, _, _| {
                range
                    .map(|index| render_split_diff_line(old.get(index), new.get(index)))
                    .collect::<Vec<_>>()
            })
            .track_scroll(scroll)
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(THEME.colors.canvas)
            .into_any_element()
        }
        _ => div().child("Preparing diff…").into_any_element(),
    }
}

fn render_split_diff_line(
    old: Option<&HighlightedDiffLine>,
    new: Option<&HighlightedDiffLine>,
) -> AnyElement {
    div()
        .w_full()
        .min_w_0()
        .h(THEME.type_scale.line_body)
        .flex()
        .child(
            div()
                .w_1_2()
                .min_w_0()
                .child(render_optional_diff_line(old)),
        )
        .child(
            div()
                .w_1_2()
                .min_w_0()
                .border_l(THEME.border)
                .border_color(THEME.colors.border)
                .child(render_optional_diff_line(new)),
        )
        .into_any_element()
}

fn render_optional_diff_line(line: Option<&HighlightedDiffLine>) -> AnyElement {
    line.map_or_else(
        || div().size_full().bg(THEME.colors.canvas).into_any_element(),
        render_diff_line,
    )
}

fn render_diff_line(line: &HighlightedDiffLine) -> AnyElement {
    let background = match line.kind {
        DiffLineKind::Context => THEME.colors.canvas,
        DiffLineKind::Addition => THEME.colors.diff_added,
        DiffLineKind::Deletion => THEME.colors.diff_deleted,
    };
    div()
        .w_full()
        .min_w_0()
        .h(THEME.type_scale.line_body)
        .overflow_hidden()
        .whitespace_nowrap()
        .px(THEME.space.xs)
        .bg(background)
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small)
        .line_height(THEME.type_scale.line_body)
        .text_color(THEME.colors.text)
        .child(line.text.element())
        .into_any_element()
}
