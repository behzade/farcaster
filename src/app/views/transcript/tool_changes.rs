use std::rc::Rc;

use gpui::{
    App, Div, ElementId, InteractiveElement as _, ParentElement as _, Role, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window, div,
};
use gpui_base::GlobalState;

use crate::{
    app::ui::{
        primitives::{AppTooltip as _, activates_button},
        theme::{MONO_FONT_FAMILY, theme},
    },
    conversation::ToolPresentation,
};

pub(super) fn title_row(
    id: impl Into<ElementId>,
    label: String,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    activation_row(id, label, move |_, window, cx| on_press(window, cx))
}

pub(super) fn file_row(
    id: impl Into<ElementId>,
    label: String,
    diff_enabled: bool,
    on_press: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let label = if diff_enabled {
        format!("{label} · ⌥ Open diff")
    } else {
        label
    };
    let tooltip = label.clone();
    activation_row(id, label, move |alt, window, cx| {
        on_press(diff_enabled && alt, window, cx)
    })
    .app_tooltip(tooltip)
}

pub(super) fn activation_row(
    id: impl Into<ElementId>,
    label: String,
    on_press: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let press = Rc::new(on_press);
    let click = press.clone();
    crate::app::ui::grid::row()
        .id(id)
        .rounded(theme().radius)
        .role(Role::Button)
        .aria_label(label)
        .tab_index(0)
        .cursor_pointer()
        .hover(|row| row.bg(theme().colors.highlight))
        .focus_visible(|row| {
            row.shadow(crate::app::ui::primitives::focus_outline(
                theme().colors.indicator,
            ))
        })
        .on_mouse_down(gpui::MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            GlobalState::suppress_text_selection(cx);
        })
        .on_click(move |event, window, cx| click(event.modifiers().alt, window, cx))
        .on_key_down(move |event, window, cx| {
            if activates_button(event) {
                cx.stop_propagation();
                press(false, window, cx);
            }
        })
}

pub(super) fn file_label(
    path: &str,
    project: Option<&std::path::Path>,
    home: Option<&std::path::Path>,
) -> String {
    let path = file_path(path, project);
    if let Some(relative) = project.and_then(|project| path.strip_prefix(project).ok()) {
        return relative.display().to_string();
    }
    if let Some(relative) = home.and_then(|home| path.strip_prefix(home).ok()) {
        return format!("~/{}", relative.display());
    }
    path.display().to_string()
}

pub(super) fn file_path(path: &str, project: Option<&std::path::Path>) -> std::path::PathBuf {
    use path_clean::PathClean as _;
    let path = std::path::Path::new(path);
    if path.is_relative() {
        project.map_or_else(|| path.to_path_buf(), |project| project.join(path))
    } else {
        path.to_path_buf()
    }
    .clean()
}

pub(super) fn file_summary(presentation: &ToolPresentation, label: String) -> Div {
    div()
        .min_w_0()
        .flex_1()
        .flex()
        .items_center()
        .gap(theme().space.xs)
        .font_family(MONO_FONT_FAMILY)
        .text_size(theme().type_scale.body_small)
        .child(
            div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(theme().colors.text)
                .child(label),
        )
        .children(change_counts(
            presentation.counts(),
            crate::app::ui::primitives::DiffColumns::for_counts([presentation.counts()]),
        ))
}

pub(super) fn change_counts(
    counts: (usize, usize),
    columns: crate::app::ui::primitives::DiffColumns,
) -> impl Iterator<Item = Div> {
    crate::app::ui::primitives::diff_count_columns(Some(counts), false, columns)
}

#[cfg(test)]
#[path = "tool_changes_tests.rs"]
mod tests;
