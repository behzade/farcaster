//! Focus policy shared by app overlays and keyboard-only traversal.
use gpui::{App, FocusHandle, KeyDownEvent, Window};

#[derive(PartialEq)]
pub(crate) enum Restoration {
    Preserved,
    Target,
    Fallback,
}

/// Restore a surviving return target only while the closing surface still owns
/// input. A menu action may already have focused another dialog or an editor.
pub(crate) fn restore(
    target: Option<FocusHandle>,
    closing: &FocusHandle,
    root: &FocusHandle,
    fallback: FocusHandle,
    window: &mut Window,
    cx: &mut App,
) -> Restoration {
    if let Some(current) = window.focused(cx)
        && !closing.contains(&current, window)
        && target.as_ref() != Some(&current)
    {
        return Restoration::Preserved;
    }
    if let Some(target) =
        target.filter(|target| root.contains(target, window) && !closing.contains(target, window))
    {
        target.focus(window, cx);
        Restoration::Target
    } else {
        fallback.focus(window, cx);
        Restoration::Fallback
    }
}

/// Called only for unhandled keys, after input/menu action bindings. In a modal
/// the bubble phase lets the innermost dialog own traversal, not a global trap.
pub(crate) fn traverse_tab(
    event: &KeyDownEvent,
    scope: Option<&FocusHandle>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let modifiers = event.keystroke.modifiers;
    if event.keystroke.key != "tab"
        || modifiers.control
        || modifiers.platform
        || modifiers.alt
        || modifiers.function
    {
        return false;
    }
    let mut visited = Vec::new();
    loop {
        if modifiers.shift {
            window.focus_prev(cx);
        } else {
            window.focus_next(cx);
        }
        let Some(scope) = scope else { break };
        if scope.contains_focused(window, cx) {
            break;
        }
        let current = window.focused(cx);
        if visited.contains(&current) {
            scope.focus(window, cx);
            break;
        }
        visited.push(current);
    }
    window.prevent_default();
    cx.stop_propagation();
    true
}

#[cfg(test)]
mod tests {
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
                // New dialog focus may precede its first render.
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
}
