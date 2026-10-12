use gpui::{AnyWindowHandle, App, Window};

#[derive(Default)]
struct Popups(Vec<(AnyWindowHandle, AnyWindowHandle)>);
impl gpui::Global for Popups {}

pub(in crate::app::views) fn register(
    owner: AnyWindowHandle,
    popup: AnyWindowHandle,
    cx: &mut App,
) {
    if !cx.has_global::<Popups>() {
        cx.set_global(Popups::default());
    }
    let live = cx.windows();
    let popups = &mut cx.global_mut::<Popups>().0;
    popups.retain(|(_, popup)| live.contains(popup));
    popups.push((owner, popup));
}

// xdg-shell requires clients to destroy their popups from top to bottom.
// A notice may expire or a lower panel may resize while a menu sits above it.
pub(in crate::app::views) fn close(window: &mut Window, cx: &mut App) {
    let current = window.window_handle();
    let above = cx
        .try_global::<Popups>()
        .and_then(|popups| {
            let index = popups.0.iter().position(|(_, popup)| *popup == current)?;
            let owner = popups.0[index].0;
            Some(
                popups.0[index + 1..]
                    .iter()
                    .rev()
                    .filter_map(|(parent, popup)| (*parent == owner).then_some(*popup))
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or_default();
    for popup in above {
        super::keyboard::unregister(popup, cx);
        let _ = popup.update(cx, |_, window, _| window.remove_window());
    }
    if cx.has_global::<Popups>() {
        let live = cx.windows();
        cx.global_mut::<Popups>()
            .0
            .retain(|(_, popup)| *popup != current && live.contains(popup));
    }
    super::keyboard::unregister(current, cx);
    window.remove_window();
}
