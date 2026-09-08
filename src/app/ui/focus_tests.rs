use super::*;
use gpui::{InteractiveElement as _, ParentElement as _, div, point, px, size};

#[gpui::test]
fn restore_keeps_new_owners_and_falls_back_for_removed_targets(cx: &mut gpui::TestAppContext) {
    let cx = cx.add_empty_window();
    let (root, owner, dialog, next) = cx.update(|_, cx| {
        (
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
            cx.focus_handle(),
        )
    });
    for owner_visible in [true, false] {
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(400.0), px(300.0)),
            |_, _| {
                div()
                    .track_focus(&root)
                    .children(owner_visible.then(|| div().track_focus(&owner)))
                    .child(div().track_focus(&dialog))
            },
        );
        cx.update(|window, cx| {
            dialog.focus(window, cx);
            restore(
                Some(owner.clone()),
                &dialog,
                &root,
                root.clone(),
                window,
                cx,
            );
            assert!(if owner_visible { &owner } else { &root }.is_focused(window));
            next.focus(window, cx);
            restore(
                Some(owner.clone()),
                &dialog,
                &root,
                root.clone(),
                window,
                cx,
            );
            assert!(next.is_focused(window));
        });
    }
}
