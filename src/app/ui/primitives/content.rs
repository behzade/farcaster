use crate::app::ui::theme::theme;
use gpui::{
    Div, FontWeight, InteractiveElement as _, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
};

pub(crate) fn panel() -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(theme().radius)
        .border(theme().border)
        .border_color(theme().colors.border)
        .bg(theme().colors.panel)
}

#[derive(Clone, Copy)]
pub(crate) struct DiffColumns {
    added: gpui::Pixels,
    removed: gpui::Pixels,
}

impl DiffColumns {
    /// Size once for the whole tree, including totals of collapsed folders.
    pub(crate) fn for_counts(counts: impl IntoIterator<Item = (usize, usize)>) -> Self {
        let totals = counts
            .into_iter()
            .fold((0usize, 0usize), |(a, r), (added, removed)| {
                (a.saturating_add(added), r.saturating_add(removed))
            });
        let width = |n: usize| theme().size((8.0 * (n.to_string().len() + 1) as f32).max(36.0));
        Self {
            added: width(totals.0),
            removed: width(totals.1),
        }
    }
}

pub(crate) fn folder_change_summary(
    count: usize,
    counts: Option<(usize, usize)>,
    columns: DiffColumns,
) -> Div {
    div()
        .flex_none()
        .whitespace_nowrap()
        .flex()
        .items_center()
        .gap(theme().space.xs)
        .font_weight(FontWeight::MEDIUM)
        .child(div().text_color(theme().colors.muted).child(format!(
            "{count} {}",
            if count == 1 { "file" } else { "files" }
        )))
        .children(diff_count_columns(counts, true, columns))
}

pub(crate) fn diff_count_columns(
    counts: Option<(usize, usize)>,
    show_zero: bool,
    columns: DiffColumns,
) -> impl Iterator<Item = Div> {
    [
        (
            counts.map(|(added, _)| added),
            "+",
            theme().colors.success,
            columns.added,
        ),
        (
            counts.map(|(_, removed)| removed),
            "−",
            theme().colors.error,
            columns.removed,
        ),
    ]
    .into_iter()
    .map(move |(count, sign, color, width)| {
        div()
            .debug_selector(move || {
                if sign == "+" {
                    "diff-added-column"
                } else {
                    "diff-removed-column"
                }
                .into()
            })
            .w(width)
            .flex_none()
            .text_right()
            .text_color(color)
            .when(show_zero || count != Some(0), |slot| {
                slot.child(count.map_or_else(|| format!("{sign}—"), |n| format!("{sign}{n}")))
            })
    })
}

pub(crate) fn section_heading(title: impl Into<SharedString>) -> impl gpui::IntoElement {
    let title = title.into();
    div()
        .id(title.clone())
        .role(Role::Heading)
        .aria_label(title.clone())
        .aria_level(2)
        .text_size(theme().type_scale.reading)
        .line_height(theme().type_scale.line_reading)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme().colors.text)
        .child(title)
}

/// Pane sections sit below item titles in the type hierarchy.
pub(crate) fn sidebar_heading(title: impl Into<SharedString>) -> impl gpui::IntoElement {
    let title = title.into();
    div()
        .id(title.clone())
        .role(Role::Heading)
        .aria_label(title.clone())
        .aria_level(2)
        .text_size(theme().type_scale.body_small)
        .line_height(theme().type_scale.line_body)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme().colors.muted)
        .child(title)
}
