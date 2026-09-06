use gpui::{IntoElement, Styled as _, WeakEntity};

use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

use crate::app::{
    FarcasterApp,
    ui::{
        primitives::{ButtonTone, dropdown_button},
        theme::THEME,
    },
};

pub(super) fn harness_selector(
    harness: &str,
    entity: WeakEntity<FarcasterApp>,
) -> impl IntoElement {
    let backends = crate::agents::backend_statuses()
        .into_iter()
        .filter(|backend| backend.available)
        .collect::<Vec<_>>();
    let selected = harness.to_owned();
    let label = backends
        .iter()
        .find(|backend| backend.id == harness)
        .map(|backend| backend.name.to_owned())
        .unwrap_or_else(|| harness.to_owned());
    dropdown_button("draft-harness", label, ButtonTone::Quiet, true)
        .text_color(THEME.colors.text)
        .dropdown_menu_with_anchor(gpui::Anchor::BottomLeft, move |mut menu, _, _| {
            for backend in &backends {
                let target = backend.id.clone();
                let entity = entity.clone();
                menu = menu.item(
                    PopupMenuItem::new(backend.name.clone())
                        .checked(backend.id == selected)
                        .on_click(move |_, window, cx| {
                            let _ = entity.update(cx, |this, cx| {
                                this.change_draft_harness(target.clone(), window, cx);
                            });
                        }),
                );
            }
            menu
        })
}
