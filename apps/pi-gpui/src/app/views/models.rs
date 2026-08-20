use gpui::{
    Anchor, AnyElement, FontWeight, IntoElement as _, ParentElement as _, Styled as _, WeakEntity,
    div, prelude::FluentBuilder as _, px,
};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

use super::super::PiApp;
use crate::{
    primitives::{ButtonTone, dropdown_button},
    theme::{MONO_FONT_FAMILY, THEME},
};

const PROVIDER_CONTROL_WIDTH: f32 = 150.0;
const MODEL_CONTROL_WIDTH: f32 = 210.0;
const EFFORT_CONTROL_WIDTH: f32 = 120.0;

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
            bounded_label(&selected_provider, 14)
        };
        let model_button_label = selected_model.map_or_else(|| "Model".into(), |_| model_label);
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

        let provider = dropdown_button(
            "select-provider",
            provider_button_label,
            ButtonTone::Quiet,
            !providers.is_empty(),
        )
        .w_full()
        .font_family(MONO_FONT_FAMILY)
        .text_color(THEME.colors.text)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            let mut menu = menu
                .min_w(px(PROVIDER_CONTROL_WIDTH))
                .max_h(px(420.0))
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
            menu
        });
        let model = dropdown_button(
            "select-model",
            model_button_label,
            ButtonTone::Quiet,
            !provider_models.is_empty(),
        )
        .w_full()
        .font_family(MONO_FONT_FAMILY)
        .text_color(THEME.colors.text)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            let mut menu = menu
                .min_w(px(MODEL_CONTROL_WIDTH))
                .max_h(px(480.0))
                .label("Model");
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
            .min_w_0()
            .flex_1()
            .flex()
            .items_stretch()
            .child(control_cell(
                "PROVIDER",
                PROVIDER_CONTROL_WIDTH,
                provider.into_any_element(),
                true,
            ))
            .child(control_cell(
                "MODEL",
                MODEL_CONTROL_WIDTH,
                model.into_any_element(),
                true,
            ))
            .child(control_cell(
                "EFFORT",
                EFFORT_CONTROL_WIDTH,
                effort_selector(effort, &efforts, effort_entity),
                false,
            ))
            .into_any_element()
    }
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
    .w_full()
    .font_family(MONO_FONT_FAMILY)
    .text_color(effort_color(selected.unwrap_or("off")))
    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
        let mut menu = menu
            .min_w(px(EFFORT_CONTROL_WIDTH))
            .max_h(px(420.0))
            .label("Effort");
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

fn control_cell(
    label: &'static str,
    width: f32,
    control: AnyElement,
    separated: bool,
) -> AnyElement {
    div()
        .min_w(px(width))
        .flex_1()
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(2.0))
        .px(THEME.space.md)
        .when(separated, |cell| {
            cell.border_r(THEME.border)
                .border_color(THEME.colors.border)
        })
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .font_weight(FontWeight::MEDIUM)
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

fn bounded_label(value: &str, max: usize) -> String {
    let mut label = value.chars().take(max).collect::<String>();
    if value.chars().count() > max {
        label.push('…');
    }
    label
}
