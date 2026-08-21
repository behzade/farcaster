//! Provider, model, and effort selectors for the composer footer.

use gpui::{
    Anchor, AnyElement, FontWeight, IntoElement as _, ParentElement as _, Styled as _, WeakEntity,
    div,
};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

use super::{composer_footer::separator, super::PiApp};
use crate::{
    primitives::{ButtonTone, dropdown_button},
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

    let provider = dropdown_button(
        "select-provider",
        provider_label,
        ButtonTone::Quiet,
        true,
    )
    .flex_none()
    .font_family(MONO_FONT_FAMILY)
    .text_color(THEME.colors.text)
    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
        let mut menu = menu
            .max_h(THEME.layout.dialog_max_height)
            .label("Provider");
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
        menu.item(PopupMenuItem::new("+ Add provider…").on_click(move |_, _, cx| {
            let _ = add_provider_entity.update(cx, |this, _| this.add_provider());
        }))
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
            menu = menu.item(PopupMenuItem::new(model.name.clone()).on_click(
                move |_, _, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.select_model(&target, cx);
                    });
                },
            ));
        }
        menu
    });

    div()
        .flex_none()
        .flex()
        .items_center()
        .child(inline_control("provider:", provider.into_any_element()))
        .child(separator())
        .child(inline_control("model:", model.into_any_element()))
        .child(separator())
        .child(inline_control(
            "effort:",
            effort_selector(effort, &efforts, entity),
        ))
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

fn inline_control(label: &'static str, control: AnyElement) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small)
        .child(
            div()
                .flex_none()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(THEME.colors.subtle)
                .child(label),
        )
        .child(control)
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
