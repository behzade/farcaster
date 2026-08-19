use gpui::{
    Anchor, AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div, px,
};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

use super::super::PiApp;
use crate::{
    primitives::{ButtonTone, button, dropdown_button},
    theme::THEME,
};

impl PiApp {
    pub(super) fn render_composer_controls(&self, entity: WeakEntity<Self>) -> AnyElement {
        let identity = self.snapshot.session_identity();
        let selected_model = identity.model;
        let selected_provider = identity
            .provider
            .map(str::to_owned)
            .or_else(|| {
                self.snapshot
                    .models
                    .first()
                    .map(|model| model.provider.clone())
            })
            .unwrap_or_else(|| "Provider".into());
        let model_label = selected_model
            .map(|model| bounded_label(&model.name, 24))
            .unwrap_or_else(|| "Model".into());
        let mut providers = self
            .snapshot
            .models
            .iter()
            .map(|model| model.provider.clone())
            .collect::<Vec<_>>();
        providers.sort();
        providers.dedup();
        let provider_button_label = if providers.is_empty() {
            "Provider".into()
        } else {
            format!("Provider: {}", bounded_label(&selected_provider, 14))
        };
        let model_button_label =
            selected_model.map_or_else(|| "Model".into(), |_| format!("Model: {model_label}"));
        let provider_models = self
            .snapshot
            .models
            .iter()
            .filter(|model| model.provider == selected_provider)
            .cloned()
            .collect::<Vec<_>>();
        let effort = identity.effort;
        let efforts = self.snapshot.thinking_levels.clone();
        let provider_entity = entity.clone();
        let model_entity = entity.clone();
        let effort_entity = entity;

        div()
            .min_w_0()
            .flex_1()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(THEME.space.xs)
            .child(
                button(
                    "select-provider",
                    provider_button_label,
                    ButtonTone::Neutral,
                    !providers.is_empty(),
                    |_, _| {},
                )
                .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                    let mut menu = menu.min_w(px(180.0)).max_h(px(420.0)).label("Provider");
                    for provider in &providers {
                        let target = provider.clone();
                        let entity = provider_entity.clone();
                        menu = menu.item(PopupMenuItem::new(provider.clone()).on_click(
                            move |_, _, cx| {
                                let _ = entity.update(cx, |this, cx| {
                                    this.select_provider(&target, cx);
                                });
                            },
                        ));
                    }
                    menu
                }),
            )
            .child(
                button(
                    "select-model",
                    model_button_label,
                    ButtonTone::Neutral,
                    !provider_models.is_empty(),
                    |_, _| {},
                )
                .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                    let mut menu = menu.min_w(px(260.0)).max_h(px(480.0)).label("Model");
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
                }),
            )
            .child(effort_selector(effort, &efforts, effort_entity))
            .into_any_element()
    }
}

fn effort_selector(
    selected: Option<&str>,
    efforts: &[String],
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let label = selected.map_or_else(
        || "Effort".into(),
        |level| format!("Effort: {}", effort_label(level)),
    );
    let efforts = efforts.to_vec();

    dropdown_button(
        "select-effort",
        label,
        ButtonTone::Quiet,
        !efforts.is_empty(),
    )
    .text_color(effort_color(selected.unwrap_or("off")))
    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
        let mut menu = menu.min_w(px(160.0)).max_h(px(420.0)).label("Effort");
        for effort in &efforts {
            let target = effort.clone();
            let click_entity = entity.clone();
            menu = menu.item(
                PopupMenuItem::new(effort_label(effort)).on_click(move |_, _, cx| {
                    let _ = click_entity.update(cx, |this, cx| {
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

fn bounded_label(value: &str, max: usize) -> String {
    let mut label = value.chars().take(max).collect::<String>();
    if value.chars().count() > max {
        label.push('…');
    }
    label
}
