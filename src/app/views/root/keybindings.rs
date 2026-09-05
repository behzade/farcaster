use gpui::{IntoElement, ParentElement as _, Styled as _, div};
use gpui_component::kbd::Kbd;

use crate::app::ui::theme::THEME;

pub(super) fn render_help() -> impl IntoElement {
    let shortcuts = crate::app::ui::navigation::help_shortcuts()
        .into_iter()
        .map(|(section, key, label)| (section.to_owned(), key, label.to_owned()))
        .chain(
            crate::app::ui::keybindings::registry()
                .into_iter()
                .filter(|shortcut| shortcut.show_in_help)
                .map(|shortcut| {
                    (
                        format!("Direct · {}", shortcut.section),
                        shortcut.keystroke,
                        shortcut.label.to_owned(),
                    )
                }),
        );
    let mut content = div()
        .flex()
        .flex_col()
        .gap(THEME.space.md)
        .p(THEME.space.md)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(THEME.space.xs)
                .pb(THEME.space.sm)
                .border_b(THEME.border)
                .border_color(THEME.colors.border)
                .child(
                    div()
                        .text_size(THEME.type_scale.display)
                        .text_color(THEME.colors.text)
                        .child("Keyboard shortcuts"),
                )
                .child(
                    div()
                        .text_size(THEME.type_scale.body_small)
                        .text_color(THEME.colors.muted)
                        .child("Ctrl+G returns to chat normal. Space begins a leader sequence. Composer and embedded tools keep their own typing keys."),
                ),
        );
    let mut current_section = String::new();
    let mut section = None;

    for (section_name, keystroke, label) in shortcuts {
        if section_name != current_section {
            if let Some(previous) = section.take() {
                content = content.child(previous);
            }
            current_section = section_name.clone();
            section = Some(
                div().flex().flex_col().gap(THEME.space.xs).child(
                    div()
                        .mb(THEME.space.xs)
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.accent)
                        .child(section_name),
                ),
            );
        }

        let keys = div().flex().items_center().gap(THEME.space.xs).children(
            keystroke.split_whitespace().map(|key| {
                Kbd::new(gpui::Keystroke::parse(key).expect("registered shortcut must parse"))
            }),
        );
        section = section.map(|section| {
            section.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(THEME.space.md)
                    .min_h(THEME.controls.utility_row)
                    .px(THEME.space.sm)
                    .py(THEME.space.xs)
                    .rounded(THEME.radius)
                    .bg(THEME.colors.surface)
                    .child(label)
                    .child(keys),
            )
        });
    }

    if let Some(section) = section {
        content = content.child(section);
    }
    content
}
