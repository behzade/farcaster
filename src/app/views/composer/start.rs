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
    let backends = crate::agents::backend_statuses();
    let selected = harness.to_owned();
    let label = backends
        .iter()
        .find(|backend| backend.id == harness)
        .map_or_else(
            || {
                if harness.is_empty() {
                    "Choose a backend".to_owned()
                } else {
                    "Unavailable backend".to_owned()
                }
            },
            |backend| {
                if backend.available {
                    backend.name.clone()
                } else {
                    format!("{} (not installed)", backend.name)
                }
            },
        );
    dropdown_button("draft-harness", label, ButtonTone::Quiet, true)
        .text_color(THEME.colors.text)
        .dropdown_menu_with_anchor(gpui::Anchor::BottomLeft, move |mut menu, _, _| {
            for backend in &backends {
                let target = backend.id.clone();
                let entity = entity.clone();
                let label = if backend.available {
                    backend.name.clone()
                } else {
                    format!(
                        "{} — not installed (expected: {})",
                        backend.name,
                        backend.program.display()
                    )
                };
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .checked(backend.id == selected)
                        .disabled(!backend.available)
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
