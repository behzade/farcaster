use gpui::{IntoElement, ParentElement as _, Styled as _, div};
use gpui_component::kbd::Kbd;

use crate::app::ui::theme::THEME;

pub(super) fn render_help() -> impl IntoElement {
    let shortcuts = crate::app::ui::navigation::help_shortcuts()
        .into_iter()
        .map(|(section, key, label)| (section.to_owned(), key, label))
        .chain(
            crate::app::ui::keybindings::registry()
                .into_iter()
                .filter(|shortcut| shortcut.show_in_help)
                .map(|shortcut| {
                    (
                        format!("Direct · {}", shortcut.section),
                        shortcut.keystroke,
                        shortcut.label,
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
                        .child("Ctrl+G activates app keys for 1 second; double Ctrl+G returns to the chat composer. In chat, Ctrl+F/B scroll a page and Ctrl+U/D scroll half a page without moving focus."),
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

        section = section.map(|section| section.child(shortcut_row(&keystroke, label)));
    }

    if let Some(section) = section {
        content = content.child(section);
    }
    content
}

fn shortcut_row(keystroke: &str, label: &str) -> impl IntoElement {
    use gpui::InteractiveElement as _;
    let keys = div()
        .debug_selector(|| "shortcut-keys".into())
        .flex()
        .flex_wrap()
        .flex_none()
        .max_w_full()
        .items_center()
        .gap(THEME.space.xs)
        .children(keystroke.split_whitespace().map(|key| {
            Kbd::new(gpui::Keystroke::parse(key).expect("registered shortcut must parse"))
        }));
    div()
        .debug_selector(|| "shortcut-row".into())
        .w_full()
        .min_w_0()
        .flex()
        .items_center()
        .justify_between()
        .flex_wrap()
        .gap(THEME.space.md)
        .min_h(THEME.controls.utility_row)
        .px(THEME.space.sm)
        .py(THEME.space.xs)
        .rounded(THEME.radius)
        .bg(THEME.colors.surface)
        .child(
            div()
                .debug_selector(|| "shortcut-label".into())
                .min_w_0()
                .max_w_full()
                .child(label.to_owned()),
        )
        .child(keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px, size};

    #[gpui::test]
    fn long_labels_and_multi_chord_keys_fit_narrow_help(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let cx = cx.add_empty_window();
        for width in [240.0, 320.0, 488.0] {
            for key in [
                "ctrl-g ctrl-g",
                "cmd-g cmd-g",
                "ctrl-g space j",
                "cmd-shift-n",
            ] {
                cx.draw(
                    point(px(0.0), px(0.0)),
                    size(px(width), px(300.0)),
                    |_, _| {
                        div().w(px(width)).child(shortcut_row(
                            key,
                            "Activate app keys without changing keyboard focus",
                        ))
                    },
                );
                let row = cx.debug_bounds("shortcut-row").unwrap();
                for selector in ["shortcut-keys", "shortcut-label"] {
                    let bounds = cx.debug_bounds(selector).unwrap();
                    assert!(bounds.left() >= row.left());
                    assert!(
                        bounds.right() <= row.right(),
                        "{key} at {width}: {bounds:?} > {row:?}"
                    );
                    assert!(bounds.bottom() <= row.bottom());
                }
            }
        }
    }
}
