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
    primitives::{ButtonTone, button, icon_button, section_heading},
    session_changes::FileChangeKind,
    syntax_highlight::{DiffLineKind, HighlightedDiff, HighlightedText},
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
    scroll: &ScrollHandle,
) -> AnyElement {
    let Some(syntax) = syntax else {
        return div().child("Preparing diff…").into_any_element();
    };
    match (mode, syntax) {
        (FullDiffMode::Unified, HighlightedDiff::Unified(text)) => scrollable_diff(
            "full-unified-diff",
            render_diff_document("full-unified", text),
            scroll,
        ),
        (FullDiffMode::Split, HighlightedDiff::Split { old, new }) => scrollable_diff(
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
                        .child(render_diff_document("full-split-old", old)),
                )
                .child(
                    div()
                        .w_1_2()
                        .min_w_0()
                        .child(render_diff_document("full-split-new", new)),
                ),
            scroll,
        ),
        _ => div().child("Preparing diff…").into_any_element(),
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

fn render_diff_document(id: &'static str, text: &HighlightedText) -> AnyElement {
    div()
        .id(id)
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .whitespace_normal()
        .py(THEME.space.xs)
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small)
        .line_height(THEME.type_scale.line_body)
        .text_color(THEME.colors.text)
        .children(text.diff_lines().into_iter().map(|(kind, line)| {
            let background = match kind {
                DiffLineKind::Context => THEME.colors.canvas,
                DiffLineKind::Addition => THEME.colors.diff_added,
                DiffLineKind::Deletion => THEME.colors.diff_deleted,
            };
            div()
                .w_full()
                .min_w_0()
                .min_h(THEME.type_scale.line_body)
                .px(THEME.space.xs)
                .bg(background)
                .child(line.element())
        }))
        .into_any_element()
}
