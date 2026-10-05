use super::*;
use gpui::{point, px, size};

#[test]
fn command_help_groups_alternatives_and_hides_aliases() {
    use gpui::Action as _;
    let rows_for = |action| {
        command_help_rows(
            crate::app::ui::keybindings::registry()
                .into_iter()
                .find(|command| command.action.name() == action)
                .expect("command"),
        )
    };
    assert_eq!(
        rows_for(crate::app::ComposerCompletionPrevious.name()),
        vec![(
            "Composer".into(),
            vec!["ctrl-p".into(), "shift-tab".into()],
            "Previous completion",
        )]
    );
    let help = rows_for(crate::app::ShowKeybindings.name());
    assert_eq!(help.len(), 1);
    assert_eq!(
        help[0].1,
        [crate::app::ui::keybindings::application_key("shift-/")]
    );
    assert!(rows_for(crate::app::ui::keyboard::CopySelection.name()).is_empty());
}

#[gpui::test]
fn long_labels_and_multi_chord_keys_fit_narrow_help(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    for width in [240.0, 320.0, 488.0] {
        for keys in [
            vec!["ctrl-g ctrl-g".to_owned()],
            vec!["cmd-g cmd-g".to_owned()],
            vec!["ctrl-g j".to_owned()],
            vec!["cmd-shift-n".to_owned()],
            vec!["ctrl-p".to_owned(), "shift-tab".to_owned()],
            vec!["cmd-g cmd-g".to_owned(), "ctrl-g ctrl-g".to_owned()],
        ] {
            cx.draw(
                point(px(0.0), px(0.0)),
                size(px(width), px(300.0)),
                |_, _| {
                    div().w(px(width)).child(shortcut_row(
                        &keys,
                        "Activate app keys without changing keyboard focus",
                    ))
                },
            );
            let row = cx
                .debug_bounds("shortcut-row")
                .expect("test operation should succeed");
            for selector in ["shortcut-keys", "shortcut-label"] {
                let bounds = cx
                    .debug_bounds(selector)
                    .expect("test operation should succeed");
                assert!(bounds.left() >= row.left());
                assert!(
                    bounds.right() <= row.right(),
                    "{keys:?} at {width}: {bounds:?} > {row:?}"
                );
                assert!(bounds.bottom() <= row.bottom());
            }
        }
    }
}

#[gpui::test]
fn voice_shortcut_help_renders_right_shift_and_custom_binding(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    for key in ["right-shift", "ctrl-alt-v"] {
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(520.0), px(800.0)),
            |_, _| render_help(key).into_any_element(),
        );
    }
}
