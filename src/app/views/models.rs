use gpui::{
    Anchor, AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div, px,
};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

use super::{super::FarcasterApp, composer_footer::separator};
use crate::{
    assets::AppIcon,
    primitives::{AppIconSize, ButtonTone, app_icon, dropdown_content_button},
    runtime::{FileAccessMode, NetworkAccessMode, PermissionLevel},
    theme::{MONO_FONT_FAMILY, THEME},
};

pub(super) fn render(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
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
    let effort = identity.effort.unwrap_or("off");
    let runtime_content = div()
        .flex()
        .items_center()
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.caption)
        .child(div().text_color(THEME.colors.muted).child(provider_label))
        .child(runtime_slash())
        .child(div().text_color(THEME.colors.text).child(model_label))
        .child(runtime_slash())
        .child(
            div()
                .text_color(effort_color(effort))
                .child(effort_label(effort)),
        );
    let provider_entity = entity.clone();
    let add_provider_entity = entity.clone();
    let model_entity = entity.clone();
    let effort_entity = entity.clone();
    let runtime = dropdown_content_button(
        "select-runtime",
        "Runtime",
        runtime_content,
        ButtonTone::Quiet,
        true,
    )
    .flex_none()
    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
        let mut menu = menu.max_h(THEME.layout.dialog_max_height).label("Runtime");
        for provider in &providers {
            let target = provider.clone();
            let entity = provider_entity.clone();
            menu = menu.item(
                PopupMenuItem::new(format!("Provider · {provider}")).on_click(move |_, _, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.select_provider(&target, cx);
                    });
                }),
            );
        }
        menu = menu.item(PopupMenuItem::new("Provider · Add…").on_click({
            let entity = add_provider_entity.clone();
            move |_, _, cx| {
                let _ = entity.update(cx, |this, _| this.add_provider());
            }
        }));
        if !provider_models.is_empty() {
            menu = menu.separator();
        }
        for model in &provider_models {
            let target = model.clone();
            let entity = model_entity.clone();
            menu = menu.item(
                PopupMenuItem::new(format!("Model · {}", model.name)).on_click(move |_, _, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.select_model(&target, cx);
                    });
                }),
            );
        }
        if !efforts.is_empty() {
            menu = menu.separator();
        }
        for effort in &efforts {
            let target = effort.clone();
            let entity = effort_entity.clone();
            menu = menu.item(
                PopupMenuItem::new(format!("Effort · {}", effort_label(effort))).on_click(
                    move |_, _, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.set_thinking_level(target.clone(), cx);
                        });
                    },
                ),
            );
        }
        menu
    });

    div()
        .flex_none()
        .flex()
        .items_center()
        .child(runtime)
        .child(separator())
        .child(permission_selector(app.snapshot.permission_level, entity))
        .into_any_element()
}

fn runtime_slash() -> AnyElement {
    div()
        .px(px(6.0))
        .text_color(THEME.colors.subtle)
        .child("/")
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

fn permission_selector(selected: PermissionLevel, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let content = div()
        .flex()
        .items_center()
        .gap(px(7.0))
        .child(permission_indicator(
            AppIcon::Folder,
            file_access_color(selected.files),
        ))
        .child(div().text_color(THEME.colors.subtle).child("/"))
        .child(permission_indicator(
            AppIcon::Globe,
            network_access_color(selected.network),
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

fn permission_indicator(resource: AppIcon, color: gpui::Rgba) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .child(app_icon(resource, AppIconSize::Inline).text_color(THEME.colors.muted))
        .child(div().size(px(7.0)).rounded_full().bg(color))
        .into_any_element()
}

fn file_access_color(mode: FileAccessMode) -> gpui::Rgba {
    match mode {
        FileAccessMode::ReadOnly => THEME.colors.link,
        FileAccessMode::Sandboxed => THEME.colors.warning,
        FileAccessMode::Full => THEME.colors.error,
    }
}

fn network_access_color(mode: NetworkAccessMode) -> gpui::Rgba {
    match mode {
        NetworkAccessMode::Sandboxed => THEME.colors.warning,
        NetworkAccessMode::Full => THEME.colors.error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_modes_have_distinct_compact_colors() {
        assert_ne!(
            file_access_color(FileAccessMode::ReadOnly),
            file_access_color(FileAccessMode::Sandboxed)
        );
        assert_ne!(
            file_access_color(FileAccessMode::Sandboxed),
            file_access_color(FileAccessMode::Full)
        );
        assert_ne!(
            network_access_color(NetworkAccessMode::Sandboxed),
            network_access_color(NetworkAccessMode::Full)
        );
    }
}
