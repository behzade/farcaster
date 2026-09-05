use std::rc::Rc;

use gpui::{
    App, Div, ElementId, InteractiveElement as _, ParentElement as _, Role, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_base::GlobalState;
use gpui_component::tooltip::Tooltip;

use crate::app::{
    ui::{
        primitives::activates_button,
        theme::{MONO_FONT_FAMILY, THEME},
    },
    views::transcript::conversation::ToolPresentation,
};

/// All tool types use the same row geometry, without an extra disclosure gutter.
pub(super) fn title_row(
    id: impl Into<ElementId>,
    label: String,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let press = Rc::new(on_press);
    let click = press.clone();
    let tooltip = label.clone();
    div()
        .id(id)
        .w_full()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .rounded(THEME.radius)
        .role(Role::Button)
        .aria_label(label)
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .tab_index(0)
        .cursor_pointer()
        .hover(|row| row.bg(THEME.colors.hover))
        .focus_visible(|row| row.border(THEME.border).border_color(THEME.colors.accent))
        .on_mouse_down(gpui::MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            GlobalState::suppress_text_selection(cx);
        })
        .on_click(move |_, window, cx| click(window, cx))
        .on_key_down(move |event, window, cx| {
            if activates_button(event) {
                cx.stop_propagation();
                press(window, cx);
            }
        })
}

pub(super) fn tool_label(label: impl Into<gpui::SharedString>) -> Div {
    div()
        .max_w(px(160.0))
        .flex_none()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(THEME.type_scale.body_small)
        .text_color(THEME.colors.muted)
        .child(label.into())
}

pub(super) fn file_label(
    path: &str,
    project: Option<&std::path::Path>,
    home: Option<&std::path::Path>,
) -> String {
    use path_clean::PathClean;
    use std::path::Path;

    let path = Path::new(path);
    let path = if path.is_relative() {
        project.map_or_else(|| path.to_path_buf(), |project| project.join(path))
    } else {
        path.to_path_buf()
    }
    .clean();
    if let Some(relative) = project.and_then(|project| path.strip_prefix(project).ok()) {
        return relative.display().to_string();
    }
    if let Some(relative) = home.and_then(|home| path.strip_prefix(home).ok()) {
        return format!("~/{}", relative.display());
    }
    path.display().to_string()
}

pub(super) fn file_summary(presentation: &ToolPresentation, label: String) -> Div {
    let (additions, deletions) = presentation.counts();
    div()
        .min_w_0()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small)
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(THEME.colors.text)
                .child(label),
        )
        .child(
            div()
                .flex_none()
                .text_color(THEME.colors.success)
                .child(format!("+{additions}")),
        )
        .child(
            div()
                .flex_none()
                .text_color(THEME.colors.error)
                .child(format!("−{deletions}")),
        )
}

#[cfg(test)]
mod tests {
    use super::file_label;
    use std::path::Path;

    #[test]
    fn file_labels_prefer_project_then_home() {
        let project = Some(Path::new("/home/user/repo"));
        let home = Some(Path::new("/home/user"));
        for (path, expected) in [
            ("/home/user/repo/src/main.rs", "src/main.rs"),
            ("src/main.rs", "src/main.rs"),
            ("../notes.txt", "~/notes.txt"),
            ("/home/user/repo-other/file.rs", "~/repo-other/file.rs"),
            ("/home/user-other/file.rs", "/home/user-other/file.rs"),
            ("/etc/config", "/etc/config"),
        ] {
            assert_eq!(file_label(path, project, home), expected, "{path}");
        }
    }
}
