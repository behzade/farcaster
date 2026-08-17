use std::path::Path;

use gpui::{
    Anchor, AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

use super::super::PiApp;
use crate::{
    primitives::{ButtonTone, button},
    theme::THEME,
};

impl PiApp {
    pub(super) fn render_composer_controls(&self, entity: WeakEntity<Self>) -> AnyElement {
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
        let model_button_label = selected_model
            .as_ref()
            .map_or_else(|| "Model".into(), |_| format!("Model: {model_label}"));
        let provider_models = self
            .snapshot
            .models
            .iter()
            .filter(|model| model.provider == selected_provider)
            .cloned()
            .collect::<Vec<_>>();
        let effort = self
            .snapshot
            .session
            .as_ref()
            .map(|state| state.thinking_level.clone());
        let effort_button_label = effort.as_deref().map_or_else(
            || "Effort".into(),
            |level| format!("Effort: {}", effort_label(level)),
        );
        let efforts = self.snapshot.thinking_levels.clone();
        let projects = self.available_projects();
        let project_entity = entity.clone();
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
            .when_some(self.editable_draft_project(), |controls, project| {
                controls.child(
                    button(
                        "select-project",
                        format!("Project: {}", bounded_label(&project_label(&project), 18)),
                        ButtonTone::Neutral,
                        !projects.is_empty(),
                        |_, _| {},
                    )
                    .dropdown_menu_with_anchor(
                        Anchor::TopRight,
                        move |menu, _, _| {
                            let mut menu = menu.min_w(px(220.0)).max_h(px(420.0)).label("Project");
                            for project in &projects {
                                let target = project.clone();
                                let entity = project_entity.clone();
                                menu =
                                    menu.item(PopupMenuItem::new(project_label(project)).on_click(
                                        move |_, _, cx| {
                                            let _ = entity.update(cx, |this, cx| {
                                                this.change_draft_project(target.clone(), cx);
                                            });
                                        },
                                    ));
                            }
                            menu
                        },
                    ),
                )
            })
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
            .child(
                button(
                    "select-effort",
                    effort_button_label,
                    ButtonTone::Neutral,
                    !efforts.is_empty(),
                    |_, _| {},
                )
                .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                    let mut menu = menu.min_w(px(140.0)).max_h(px(360.0)).label("Effort");
                    for effort in &efforts {
                        let target = effort.clone();
                        let entity = effort_entity.clone();
                        menu = menu.item(PopupMenuItem::new(effort_label(effort)).on_click(
                            move |_, _, cx| {
                                let _ = entity.update(cx, |this, cx| {
                                    this.set_thinking_level(target.clone(), cx);
                                });
                            },
                        ));
                    }
                    menu
                }),
            )
            .into_any_element()
    }
}

fn effort_label(level: &str) -> String {
    let mut characters = level.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => "Off".into(),
    }
}

fn project_label(project: &Path) -> String {
    project
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| project.display().to_string(), str::to_owned)
}

fn bounded_label(value: &str, max: usize) -> String {
    let mut label = value.chars().take(max).collect::<String>();
    if value.chars().count() > max {
        label.push('…');
    }
    label
}
