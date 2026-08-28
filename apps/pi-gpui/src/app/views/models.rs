//! Provider, model, effort, and permission selectors for the composer footer.

use gpui::{
    Anchor, AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div,
};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

use super::{super::PiApp, composer_footer::separator};
use crate::{
    assets::AppIcon,
    primitives::{
        AppIconSize, ButtonTone, app_icon, dropdown_button, dropdown_content_button,
    },
    runtime::{FileAccessMode, NetworkAccessMode, PermissionLevel},
    theme::{MONO_FONT_FAMILY, THEME},
};

pub(super) fn render(app: &PiApp, entity: WeakEntity<PiApp>) -> AnyElement {
    let identity = app.snapshot.session_identity();
    let selected_model = identity.model;
    let selected_provider = identity
        .provider
        .map(str::to_owned)
        .or_else(|| {
            app.snapshot
                .models
                .first()
                .map(|model| model.provider.clone())
        })
        .unwrap_or_else(|| "Provider".into());
    let mut providers = app
        .snapshot
        .models
        .iter()
        .map(|model| model.provider.clone())
        .collect::<Vec<_>>();
    providers.sort();
    providers.dedup();
    let provider_label = if providers.is_empty() {
        "Provider".into()
    } else {
        selected_provider.clone()
    };
    let model_label = selected_model
        .map(|model| model.name.clone())
        .unwrap_or_else(|| "Model".into());
    let provider_models = app
        .snapshot
        .models
        .iter()
        .filter(|model| model.provider == selected_provider)
        .cloned()
        .collect::<Vec<_>>();
    let efforts = app.snapshot.thinking_levels.clone();
    let effort = identity.effort;
    let provider_entity = entity.clone();
    let add_provider_entity = entity.clone();
    let model_entity = entity.clone();

    let provider = dropdown_button("select-provider", provider_label, ButtonTone::Quiet, true)
        .flex_none()
        .font_family(MONO_FONT_FAMILY)
        .text_color(THEME.colors.text)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            let mut menu = menu.max_h(THEME.layout.dialog_max_height).label("Provider");
            for provider in &providers {
                let target = provider.clone();
                let entity = provider_entity.clone();
                menu = menu.item(
                    PopupMenuItem::new(provider.clone()).on_click(move |_, _, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.select_provider(&target, cx);
                        });
                    }),
                );
            }
            if !providers.is_empty() {
                menu = menu.separator();
            }
            let add_provider_entity = add_provider_entity.clone();
            menu.item(
                PopupMenuItem::new("+ Add provider…").on_click(move |_, _, cx| {
                    let _ = add_provider_entity.update(cx, |this, _| this.add_provider());
                }),
            )
        });
    let model = dropdown_button(
        "select-model",
        model_label,
        ButtonTone::Quiet,
        !provider_models.is_empty(),
    )
    .flex_none()
    .font_family(MONO_FONT_FAMILY)
    .text_color(THEME.colors.text)
    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
        let mut menu = menu.max_h(THEME.layout.dialog_max_height).label("Model");
        for model in &provider_models {
            let target = model.clone();
            let entity = model_entity.clone();
            menu = menu.item(
                PopupMenuItem::new(model.name.clone()).on_click(move |_, _, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.select_model(&target, cx);
                    });
                }),
            );
        }
        menu
    });

    div()
        .flex_none()
        .flex()
        .items_center()
        .child(provider)
        .child(separator())
        .child(model)
        .child(separator())
        .child(effort_selector(effort, &efforts, entity.clone()))
        .child(separator())
        .child(permission_selector(app.snapshot.permission_level, entity))
        .into_any_element()
}

fn effort_selector(
    selected: Option<&str>,
    efforts: &[String],
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let label = selected.map_or_else(|| "Effort".into(), effort_label);
    let efforts = efforts.to_vec();
    dropdown_button(
        "select-effort",
        label,
        ButtonTone::Quiet,
        !efforts.is_empty(),
    )
    .flex_none()
    .font_family(MONO_FONT_FAMILY)
    .text_color(effort_color(selected.unwrap_or("off")))
    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
        let mut menu = menu.max_h(THEME.layout.dialog_max_height).label("Effort");
        for effort in &efforts {
            let target = effort.clone();
            let entity = entity.clone();
            menu = menu.item(
                PopupMenuItem::new(effort_label(effort)).on_click(move |_, _, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.set_thinking_level(target.clone(), cx);
                    });
                }),
            );
        }
        menu
    })
    .into_any_element()
}

fn effort_color(level: &str) -> gpui::Rgba {
    match level.to_ascii_lowercase().as_str() {
        "off" => THEME.colors.subtle,
        "minimal" => THEME.colors.muted,
        "low" => THEME.colors.link,
        "medium" => THEME.colors.accent,
        "high" => THEME.colors.warning,
        "xhigh" | "max" => THEME.colors.error,
        _ => THEME.colors.accent,
    }
}

fn effort_label(level: &str) -> String {
    let mut characters = level.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => "Off".into(),
    }
}

fn permission_selector(selected: PermissionLevel, entity: WeakEntity<PiApp>) -> AnyElement {
    let (file_state_icon, file_state_color) = file_access_presentation(selected.files);
    let (network_state_icon, network_state_color) = network_access_presentation(selected.network);
    let content = div()
        .flex()
        .items_center()
        .gap(THEME.space.sm)
        .child(permission_icon_pair(
            AppIcon::Folder,
            file_state_icon,
            file_state_color,
        ))
        .child(permission_icon_pair(
            AppIcon::ArrowSquareOut,
            network_state_icon,
            network_state_color,
        ));

    dropdown_content_button(
        "select-permission",
        selected.label(),
        content,
        ButtonTone::Quiet,
        true,
    )
    .flex_none()
    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
        let mut menu = menu
            .max_h(THEME.layout.dialog_max_height)
            .label("Sandbox access");
        for files in FileAccessMode::all() {
            let entity = entity.clone();
            menu = menu.item(
                PopupMenuItem::new(format!("Files · {}", files.label())).on_click(
                    move |_, _, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.set_permission_level(selected.with_files(files), cx);
                        });
                    },
                ),
            );
        }
        menu = menu.separator();
        for network in NetworkAccessMode::all() {
            let entity = entity.clone();
            menu = menu.item(
                PopupMenuItem::new(format!("Network · {}", network.label())).on_click(
                    move |_, _, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.set_permission_level(selected.with_network(network), cx);
                        });
                    },
                ),
            );
        }
        menu
    })
    .into_any_element()
}

fn permission_icon_pair(
    resource: AppIcon,
    access: AppIcon,
    access_color: gpui::Rgba,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .child(
            app_icon(resource, AppIconSize::Inline).text_color(THEME.colors.subtle),
        )
        .child(app_icon(access, AppIconSize::Inline).text_color(access_color))
        .into_any_element()
}

fn file_access_presentation(mode: FileAccessMode) -> (AppIcon, gpui::Rgba) {
    match mode {
        FileAccessMode::ReadOnly => (AppIcon::Eye, THEME.colors.link),
        FileAccessMode::Sandboxed => (AppIcon::Archive, THEME.colors.success),
        FileAccessMode::Full => (AppIcon::ArrowsOut, THEME.colors.warning),
    }
}

fn network_access_presentation(mode: NetworkAccessMode) -> (AppIcon, gpui::Rgba) {
    match mode {
        NetworkAccessMode::Sandboxed => (AppIcon::Archive, THEME.colors.success),
        NetworkAccessMode::Full => (AppIcon::ArrowsOut, THEME.colors.warning),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_modes_have_distinct_compact_icons() {
        assert_eq!(
            file_access_presentation(FileAccessMode::ReadOnly).0,
            AppIcon::Eye
        );
        assert_eq!(
            file_access_presentation(FileAccessMode::Sandboxed).0,
            AppIcon::Archive
        );
        assert_eq!(
            file_access_presentation(FileAccessMode::Full).0,
            AppIcon::ArrowsOut
        );
        assert_eq!(
            network_access_presentation(NetworkAccessMode::Sandboxed).0,
            AppIcon::Archive
        );
        assert_eq!(
            network_access_presentation(NetworkAccessMode::Full).0,
            AppIcon::ArrowsOut
        );
    }
}
