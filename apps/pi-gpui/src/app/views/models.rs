use gpui::{
    Anchor, AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Selectable as _, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    menu::{DropdownMenu as _, PopupMenuItem},
};

use super::super::PiApp;
use crate::{
    primitives::{ButtonTone, button},
    theme::THEME,
};

impl PiApp {
    pub(super) fn render_model_controls(&self, entity: WeakEntity<Self>) -> AnyElement {
        let selected_model = self
            .snapshot
            .session
            .as_ref()
            .and_then(|state| state.model.clone());
        let selected_provider = selected_model
            .as_ref()
            .map(|model| model.provider.clone())
            .or_else(|| {
                self.snapshot
                    .models
                    .first()
                    .map(|model| model.provider.clone())
            })
            .unwrap_or_else(|| "Provider".into());
        let model_label = selected_model
            .as_ref()
            .map(|model| bounded_label(&model.name, 28))
            .unwrap_or_else(|| "Model".into());
        let mut providers = self
            .snapshot
            .models
            .iter()
            .map(|model| model.provider.clone())
            .collect::<Vec<_>>();
        providers.sort();
        providers.dedup();
        let provider_models = self
            .snapshot
            .models
            .iter()
            .filter(|model| model.provider == selected_provider)
            .cloned()
            .collect::<Vec<_>>();
        let thinking = self
            .snapshot
            .session
            .as_ref()
            .map(|state| state.thinking_level.clone())
            .unwrap_or_else(|| "off".into());
        let provider_entity = entity.clone();
        let model_entity = entity.clone();

        div()
            .flex()
            .flex_col()
            .gap(THEME.space.sm)
            .child(
                div()
                    .flex()
                    .gap(THEME.space.xs)
                    .child(
                        button(
                            "select-provider",
                            bounded_label(&selected_provider, 14),
                            ButtonTone::Neutral,
                            !providers.is_empty(),
                            |_, _| {},
                        )
                        .flex_1()
                        .dropdown_menu_with_anchor(
                            Anchor::TopRight,
                            move |menu, _, _| {
                                let mut menu =
                                    menu.min_w(px(180.0)).max_h(px(420.0)).label("Provider");
                                for provider in &providers {
                                    let target = provider.clone();
                                    let entity = provider_entity.clone();
                                    menu =
                                        menu.item(PopupMenuItem::new(provider.clone()).on_click(
                                            move |_, _, cx| {
                                                let _ = entity.update(cx, |this, cx| {
                                                    this.select_provider(&target, cx);
                                                });
                                            },
                                        ));
                                }
                                menu
                            },
                        ),
                    )
                    .child(
                        button(
                            "select-model",
                            model_label,
                            ButtonTone::Neutral,
                            !provider_models.is_empty(),
                            |_, _| {},
                        )
                        .flex_1()
                        .dropdown_menu_with_anchor(
                            Anchor::TopRight,
                            move |menu, _, _| {
                                let mut menu =
                                    menu.min_w(px(260.0)).max_h(px(480.0)).label("Model");
                                for model in &provider_models {
                                    let target = model.clone();
                                    let entity = model_entity.clone();
                                    menu =
                                        menu.item(PopupMenuItem::new(model.name.clone()).on_click(
                                            move |_, _, cx| {
                                                let _ = entity.update(cx, |this, cx| {
                                                    this.select_model(&target, cx);
                                                });
                                            },
                                        ));
                                }
                                menu
                            },
                        ),
                    ),
            )
            .child(thinking_track(
                &self.snapshot.thinking_levels,
                &thinking,
                entity,
            ))
            .into_any_element()
    }
}

fn thinking_track(levels: &[String], current: &str, entity: WeakEntity<PiApp>) -> AnyElement {
    div()
        .id("thinking-track")
        .role(Role::Group)
        .aria_label(format!("Thinking: {current}"))
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child("Thinking"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(1.0))
                .children(levels.iter().enumerate().map(|(index, level)| {
                    let target = level.clone();
                    let selected = target == current;
                    let entity = entity.clone();
                    Button::new(("thinking-level", index))
                        .label(thinking_abbreviation(level))
                        .tooltip(format!("Thinking: {level}"))
                        .with_size(Size::XSmall)
                        .selected(selected)
                        .when(selected, |button| button.primary())
                        .when(!selected, |button| button.ghost())
                        .on_click(move |_, _, cx| {
                            let _ = entity.update(cx, |this, cx| {
                                this.set_thinking_level(target.clone(), cx);
                            });
                        })
                })),
        )
        .into_any_element()
}

fn thinking_abbreviation(level: &str) -> &'static str {
    match level {
        "off" => "Off",
        "minimal" => "Min",
        "low" => "Low",
        "medium" => "Med",
        "high" => "High",
        "xhigh" => "X",
        "max" => "Max",
        _ => "?",
    }
}

fn bounded_label(value: &str, max: usize) -> String {
    let mut label = value.chars().take(max).collect::<String>();
    if value.chars().count() > max {
        label.push('…');
    }
    label
}
