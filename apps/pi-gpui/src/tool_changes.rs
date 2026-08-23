//! Native presentations for file mutation tools.

mod preview;

use std::rc::Rc;

use crate::{
    assets::AppIcon,
    conversation::ToolPresentation,
    diff_element::{DiffCell, DiffElement, DiffPaintRow, DiffTone},
    primitives::{ButtonTone, icon_button},
    theme::{MONO_FONT_FAMILY, THEME},
};
use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};
use preview::{ChangeKind, PairedLine, SideLine};
pub(crate) use preview::{PreparedToolChange, prepare_edit, prepare_write};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EmbeddedDiffMode {
    Split,
    Unified,
}

pub(crate) type ExpandHandler = Rc<dyn Fn(&mut Window, &mut App)>;

pub(crate) fn render(
    presentation: &ToolPresentation,
    key: usize,
    requested_mode: EmbeddedDiffMode,
    on_expand: Option<ExpandHandler>,
) -> AnyElement {
    let (path, prepared) = match presentation {
        ToolPresentation::Edit { path, prepared, .. }
        | ToolPresentation::Write { path, prepared, .. } => (path, prepared.get()),
    };
    let Some(prepared) = prepared else {
        return render_frame(path, (0, 0), key, on_expand, None);
    };
    let counts = prepared.counts();
    let omitted = prepared.omitted();
    let body = match (requested_mode, prepared.split_rows()) {
        (EmbeddedDiffMode::Split, Some(rows)) => render_split(rows, omitted),
        _ => render_unified(prepared.unified_rows(), omitted),
    };
    render_frame(path, counts, key, on_expand, Some(body))
}

fn render_frame(
    path: &str,
    (additions, deletions): (usize, usize),
    key: usize,
    on_expand: Option<ExpandHandler>,
    body: Option<AnyElement>,
) -> AnyElement {
    div()
        .id(("tool-change", key))
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .border_y(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.canvas)
        .child(render_header(path, additions, deletions, key, on_expand))
        .children(body)
        .into_any_element()
}

fn render_header(
    path: &str,
    additions: usize,
    deletions: usize,
    key: usize,
    on_expand: Option<ExpandHandler>,
) -> impl IntoElement {
    let expand = on_expand.map(|handler| {
        icon_button(
            ("expand-tool-change", key),
            AppIcon::ArrowsOut,
            "Expand diff",
            ButtonTone::Quiet,
            move |window, cx| handler(window, cx),
        )
    });
    div()
        .w_full()
        .min_w_0()
        .h(px(34.0))
        .px(THEME.space.sm)
        .flex()
        .items_center()
        .gap(THEME.space.sm)
        .border_b(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.panel)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_family(MONO_FONT_FAMILY)
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.text)
                .child(path.to_owned()),
        )
        .child(change_count(format!("+{additions}"), THEME.colors.success))
        .child(change_count(format!("-{deletions}"), THEME.colors.error))
        .children(expand)
}

fn change_count(label: String, color: gpui::Rgba) -> impl IntoElement {
    div()
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.caption)
        .text_color(color)
        .child(label)
}

fn render_split(rows: &std::sync::Arc<Vec<PairedLine>>, omitted: usize) -> AnyElement {
    let row_count = rows.len();
    let rows = rows.clone();
    div()
        .w_full()
        .min_w_0()
        .child(DiffElement::split(
            row_count,
            px(28.0),
            px(48.0),
            move |index| {
                rows.get(index)
                    .map_or_else(DiffPaintRow::default, |row| DiffPaintRow {
                        old: row.old.as_ref().map(diff_cell),
                        new: row.new.as_ref().map(diff_cell),
                    })
            },
        ))
        .when(omitted > 0, |body| body.child(limit_hint(omitted)))
        .into_any_element()
}

fn render_unified(rows: &std::sync::Arc<Vec<SideLine>>, omitted: usize) -> AnyElement {
    let row_count = rows.len();
    let rows = rows.clone();
    div()
        .w_full()
        .min_w_0()
        .child(DiffElement::unified(
            row_count,
            px(28.0),
            px(48.0),
            move |index| rows.get(index).map(diff_cell),
        ))
        .when(omitted > 0, |body| body.child(limit_hint(omitted)))
        .into_any_element()
}

fn limit_hint(remaining: usize) -> impl IntoElement {
    div()
        .px(THEME.space.md)
        .py(THEME.space.sm)
        .border_t(THEME.border)
        .border_color(THEME.colors.border)
        .text_size(THEME.type_scale.caption)
        .text_color(THEME.colors.warning)
        .child(format!("{remaining} additional lines omitted"))
}

fn diff_cell(line: &SideLine) -> DiffCell {
    let (marker, tone) = match line.kind {
        ChangeKind::Context => (" ", DiffTone::Context),
        ChangeKind::Addition => ("+", DiffTone::Addition),
        ChangeKind::Deletion => ("-", DiffTone::Deletion),
        ChangeKind::Ellipsis => ("", DiffTone::Context),
    };
    DiffCell {
        gutter: Some(match line.number {
            Some(number) => format!("{marker}{number}"),
            None => marker.to_owned(),
        }),
        text: line.syntax.clone(),
        tone,
    }
}
