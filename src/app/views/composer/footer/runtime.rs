use gpui::{
    AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    menu::{DropdownMenu as _, PopupMenuItem},
    popover::Popover,
};

use super::separator;
use crate::app::FarcasterApp;
use crate::{
    app::ui::assets::AppIcon,
    app::ui::primitives::{AppIconSize, ButtonTone, app_icon, dropdown_content_button},
    app::ui::theme::{MONO_FONT_FAMILY, THEME},
    runtime::{ConfigurationStatus, HarnessAccessMode},
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let identity = app.snapshot.session_identity();
    let selected_model = identity.model;
    let selected_provider = identity.provider.map(str::to_owned).or_else(|| {
        app.snapshot
            .models
            .first()
            .map(|model| model.provider.clone())
    });
    let catalog_loading = selected_provider.is_none()
        && !app.snapshot.connected
        && app.snapshot.configuration_status == ConfigurationStatus::Loading;
    let provider_label = selected_provider.unwrap_or_else(|| "Provider".into());
    let model_label = selected_model.map_or_else(
        || {
            if catalog_loading {
                "Loading models…".into()
            } else {
                "Model".into()
            }
        },
        |model| model.name.clone(),
    );
    let effort = identity.effort.unwrap_or("off");
    let shows_effort = !app.snapshot.available_thinking_levels().is_empty() || effort != "off";
    let runtime_content = div()
        .flex()
        .items_center()
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.body)
        .child(div().text_color(THEME.colors.muted).child(provider_label))
        .child(runtime_slash())
        .child(div().text_color(THEME.colors.text).child(model_label))
        .when(shows_effort, |content| {
            content.child(runtime_slash()).child(
                div()
                    .text_color(effort_color(effort))
                    .child(effort_label(effort)),
            )
        });
    let runtime_entity = entity.clone();
    let content_entity = entity.clone();
    let runtime = Popover::new("runtime-popover")
        .anchor(gpui::Anchor::BottomLeft)
        .appearance(false)
        .open(app.runtime_picker.open)
        .trigger(
            dropdown_content_button(
                "select-runtime",
                "Runtime",
                runtime_content,
                ButtonTone::Neutral,
                true,
            )
            .flex_none(),
        )
        .on_open_change(move |open, window, cx| {
            let _ =
                runtime_entity.update(cx, |app, cx| app.set_runtime_picker_open(*open, window, cx));
        })
        .content(move |_, window, cx| {
            content_entity
                .update(cx, |app, cx| app.render_runtime_picker(window, cx))
                .unwrap_or_else(|_| div().into_any_element())
        });

    div()
        .flex_none()
        .flex()
        .items_center()
        .child(runtime)
        .child(separator())
        .child(access_selector(
            app.snapshot.access_mode,
            app.snapshot.available_access_modes(),
            entity,
        ))
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

fn access_selector(
    selected: HarnessAccessMode,
    supported: Vec<HarnessAccessMode>,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let content = div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .text_color(access_mode_color(selected))
        .child(app_icon(AppIcon::Shield, AppIconSize::Inline))
        .child(access_mode_label(selected));
    dropdown_content_button(
        "harness-access",
        format!("Sandbox settings: {}", access_mode_label(selected)),
        content,
        ButtonTone::Quiet,
        supported.len() > 1,
    )
    .dropdown_menu_with_anchor(gpui::Anchor::BottomLeft, move |mut menu, _, _| {
        for &target in &supported {
            let entity = entity.clone();
            menu = menu.item(
                PopupMenuItem::new(access_mode_label(target))
                    .checked(target == selected)
                    .on_click(move |_, _, cx| {
                        let _ = entity.update(cx, |this, cx| this.set_access_mode(target, cx));
                    }),
            );
        }
        menu
    })
    .into_any_element()
}

const fn access_mode_label(mode: HarnessAccessMode) -> &'static str {
    match mode {
        HarnessAccessMode::Full => "Sandbox: Off",
        HarnessAccessMode::Sandboxed => "Sandbox: On",
        HarnessAccessMode::Auto => "Sandbox: Auto",
    }
}

fn access_mode_color(mode: HarnessAccessMode) -> gpui::Rgba {
    match mode {
        HarnessAccessMode::Sandboxed => THEME.colors.muted,
        HarnessAccessMode::Auto => THEME.colors.muted,
        HarnessAccessMode::Full => THEME.colors.warning,
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
