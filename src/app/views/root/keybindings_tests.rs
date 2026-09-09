use super::*;
use gpui::{point, px, size};

#[gpui::test]
fn long_labels_and_multi_chord_keys_fit_narrow_help(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    for width in [240.0, 320.0, 488.0] {
        for key in ["ctrl-g ctrl-g", "cmd-g cmd-g", "ctrl-g j", "cmd-shift-n"] {
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
                    "{key} at {width}: {bounds:?} > {row:?}"
                );
                assert!(bounds.bottom() <= row.bottom());
            }
        }
    }
}
