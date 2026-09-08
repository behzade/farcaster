use gpui::{App, FocusHandle, KeyDownEvent, Window};

#[derive(PartialEq)]
pub(crate) enum Restoration {
    Preserved,
    Target,
    Fallback,
}

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
#[path = "focus_tests.rs"]
mod tests;
