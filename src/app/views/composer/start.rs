use crate::agents::{Backend, HarnessProfile};
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
    harness: Option<Backend>,
    selected_profile_id: Option<String>,
    profiles: Vec<HarnessProfile>,
    entity: WeakEntity<FarcasterApp>,
) -> impl IntoElement {
    let backends = crate::agents::backend_statuses();
    let label = profiles
        .iter()
        .find(|profile| Some(profile.id.as_str()) == selected_profile_id.as_deref())
        .map(|profile| profile.name.clone())
        .or_else(|| {
            backends
                .iter()
                .find(|backend| Some(backend.id) == harness)
                .map(|backend| {
                    if backend.available {
                        backend.name.clone()
                    } else {
                        format!("{} (not installed)", backend.name)
                    }
                })
        })
        .unwrap_or_else(|| {
            if harness.is_none() {
                "Choose a backend".to_owned()
            } else {
                "Unavailable backend".to_owned()
            }
        });
    dropdown_button("draft-harness", label, ButtonTone::Quiet, true)
        .text_color(THEME.colors.text)
        .dropdown_menu_with_anchor(gpui::Anchor::BottomLeft, move |mut menu, _, _| {
            for backend in &backends {
                let target = backend.id;
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
                        .checked(Some(backend.id) == harness && selected_profile_id.is_none())
                        .disabled(!backend.available)
                        .on_click(move |_, window, cx| {
                            let _ = entity.update(cx, |this, cx| {
                                this.change_draft_harness(target, window, cx);
                            });
                        }),
                );
            }
            for profile in &profiles {
                let target = profile.backend;
                let profile_id = profile.id.clone();
                let entity = entity.clone();
                let available = profile.is_selectable();
                let label = if available {
                    profile.name.clone()
                } else {
                    format!(
                        "{} — not installed: {}",
                        profile.name,
                        profile.executable.display()
                    )
                };
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .checked(Some(profile.id.as_str()) == selected_profile_id.as_deref())
                        .disabled(!available)
                        .on_click(move |_, window, cx| {
                            let _ = entity.update(cx, |this, cx| {
                                this.change_draft_harness_profile(
                                    target,
                                    profile_id.clone(),
                                    window,
                                    cx,
                                );
                            });
                        }),
                );
            }
            menu
        })
}
