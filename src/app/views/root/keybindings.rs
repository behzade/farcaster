use gpui::{IntoElement, ParentElement as _, Styled as _, div};
use gpui_component::kbd::Kbd;

use crate::app::ui::theme::THEME;

pub(super) fn render_help() -> impl IntoElement {
    let shortcuts = crate::app::ui::keybindings::registry();
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
                        .child("Navigate Farcaster without leaving the keyboard."),
                ),
        );
    let mut current_section = "";
    let mut section = None;

    for shortcut in shortcuts.iter().filter(|shortcut| shortcut.show_in_help) {
        if shortcut.section != current_section {
            if let Some(previous) = section.take() {
                content = content.child(previous);
            }
            current_section = shortcut.section;
            section = Some(
                div().flex().flex_col().gap(THEME.space.xs).child(
                    div()
                        .mb(THEME.space.xs)
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.accent)
                        .child(current_section),
                ),
            );
        }

        let keys = div()
            .flex()
            .items_center()
            .gap(THEME.space.xs)
            .child(Kbd::new(
                gpui::Keystroke::parse(&shortcut.keystroke)
                    .expect("registered shortcut must parse"),
            ));
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
                    .child(shortcut.label)
                    .child(keys),
            )
        });
    }

    if let Some(section) = section {
        content = content.child(section);
    }
    content
}
