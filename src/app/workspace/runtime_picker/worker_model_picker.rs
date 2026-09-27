use super::*;
use crate::protocol::{WorkerModelChoice, WorkerModelSelection};

fn worker_option_row(
    id: &'static str,
    label: &'static str,
    options: &[String],
    selected: Option<&str>,
    entity: gpui::WeakEntity<FarcasterApp>,
    set: fn(&mut WorkerModelPicker, Option<String>),
) -> gpui::AnyElement {
    option_row(label)
        .child(
            div().flex().flex_wrap().gap(theme().space.xs).children(
                std::iter::once(None)
                    .chain(options.iter().cloned().map(Some))
                    .enumerate()
                    .map(|(index, option)| {
                        let entity = entity.clone();
                        let chosen = selected == option.as_deref();
                        option_button(
                            (id, index),
                            option.clone().unwrap_or_else(|| "Default".into()),
                            chosen,
                            move |_, cx| {
                                let _ = entity.update(cx, |app, cx| {
                                    if let Some(worker) =
                                        app.workspace.runtime_picker.worker.as_mut()
                                    {
                                        set(worker, option.clone());
                                    }
                                    cx.notify();
                                });
                            },
                        )
                    }),
            ),
        )
        .into_any_element()
}

impl WorkerModelPicker {
    fn clear_selection(&mut self) {
        self.selected = None;
        self.effort = None;
        self.service_tier = None;
    }

    fn select(&mut self, index: usize) {
        self.clear_selection();
        self.selected = Some(index);
    }
}

impl FarcasterApp {
    fn choose_worker_picker_harness(
        &mut self,
        harness: crate::agents::Backend,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(crate::protocol::ExtensionUiRequest::WorkerModel { choices, .. }) =
            self.extensions.active.dialog.as_ref()
        else {
            return;
        };
        let Some(first) = choices.iter().find(|choice| choice.harness == harness) else {
            return;
        };
        if let Some(worker) = self.workspace.runtime_picker.worker.as_mut() {
            worker.harness = harness;
            worker.provider.clone_from(&first.provider);
            worker.clear_selection();
        }
        self.reset_model_picker_search(window, cx);
    }

    fn choose_worker_picker_provider(
        &mut self,
        provider: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(worker) = self.workspace.runtime_picker.worker.as_mut() {
            worker.provider = provider;
            worker.clear_selection();
        }
        self.reset_model_picker_search(window, cx);
    }

    fn submit_worker_model_choice(
        &mut self,
        save: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(worker) = self.workspace.runtime_picker.worker.as_ref() else {
            return;
        };
        let Some(choice) = worker.selected else {
            return;
        };
        let id = worker.id.clone();
        let valid = matches!(
            self.extensions.active.dialog.as_ref(),
            Some(crate::protocol::ExtensionUiRequest::WorkerModel { id: active, choices, .. })
                if active == &id && choices.get(choice).is_some_and(|item|
                    worker.effort.as_ref().is_none_or(|effort| item.efforts.contains(effort))
                    && worker.service_tier.as_ref().is_none_or(|tier| item.service_tiers.contains(tier)))
        );
        if !valid {
            return;
        }
        let Ok(value) = serde_json::to_string(&WorkerModelSelection {
            choice,
            effort: worker.effort.clone(),
            service_tier: worker.service_tier.clone(),
            save,
        }) else {
            return;
        };
        self.set_worker_model_picker_open(false, &id, window, cx);
        self.respond_dialog_value(id, value, window, cx);
    }

    pub(super) fn render_worker_model_picker(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let Some(worker) = self.workspace.runtime_picker.worker.as_ref() else {
            return div().into_any_element();
        };
        let Some(crate::protocol::ExtensionUiRequest::WorkerModel {
            id,
            profile,
            choices,
        }) = self.extensions.active.dialog.as_ref()
        else {
            return div().into_any_element();
        };
        if id != &worker.id {
            return div().into_any_element();
        }
        let Some(search) = self.workspace.runtime_picker.search.as_ref() else {
            return div().into_any_element();
        };
        let entity = cx.entity().downgrade();
        let mut harnesses = Vec::new();
        for choice in choices {
            if !harnesses.contains(&choice.harness) {
                harnesses.push(choice.harness);
            }
        }
        let mut providers = Vec::new();
        for choice in choices
            .iter()
            .filter(|choice| choice.harness == worker.harness)
        {
            if !providers.contains(&choice.provider) {
                providers.push(choice.provider.clone());
            }
        }
        let query = search.read(cx).value().trim().to_lowercase();
        let models = choices
            .iter()
            .enumerate()
            .filter(|(_, choice)| {
                choice.harness == worker.harness && choice.provider == worker.provider
            })
            .filter(|(_, choice)| model_matches(&choice.id, &choice.name, &query))
            .map(|(index, choice)| (index, choice.clone()))
            .collect::<Vec<(usize, WorkerModelChoice)>>();
        let selected = worker.selected.and_then(|index| choices.get(index));
        let (width, height, list_height) = layout::dimensions(
            f32::from(window.viewport_size().width),
            f32::from(window.viewport_size().height),
            models.len(),
        );
        let option_rows = selected.map_or(0, |choice| {
            usize::from(!choice.efforts.is_empty()) + usize::from(!choice.service_tiers.is_empty())
        });
        let list_height = list_height.min((height - 190.0 - option_rows as f32 * 55.0).max(32.0));
        let keyboard_models = models.clone();
        let selected_index = worker.selected;
        let highlighted = self.workspace.runtime_picker.highlighted;
        let focus = search.read(cx).focus_handle(cx);
        let keyboard_entity = entity.clone();
        let harness_entity = entity.clone();
        let provider_entity = entity.clone();
        let panel = picker_panel(
            width,
            height,
            focus,
            keyboard_models.len(),
            keyboard_entity,
            move |app, index, _, _| {
                if let Some(worker) = app.workspace.runtime_picker.worker.as_mut() {
                    worker.select(keyboard_models[index].0);
                }
            },
        );
        panel
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(theme().space.sm)
                    .p(theme().space.sm)
                    .child(div().text_color(theme().colors.muted).child("Harness"))
                    .child(
                        dropdown_button(
                            "worker-harness",
                            crate::agents::backend_display_name(worker.harness),
                            ButtonTone::Quiet,
                            harnesses.len() > 1,
                        )
                        .dropdown_menu(move |mut menu, _, _| {
                            for harness in &harnesses {
                                let harness = *harness;
                                let entity = harness_entity.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(crate::agents::backend_display_name(
                                        harness,
                                    ))
                                    .on_click(
                                        move |_, window, cx| {
                                            let _ = entity.update(cx, |app, cx| {
                                                app.choose_worker_picker_harness(
                                                    harness, window, cx,
                                                )
                                            });
                                        },
                                    ),
                                );
                            }
                            menu
                        }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(theme().space.sm)
                    .px(theme().space.sm)
                    .pb(theme().space.sm)
                    .child(div().text_color(theme().colors.muted).child("Provider"))
                    .child(
                        dropdown_button(
                            "worker-provider",
                            worker.provider.clone(),
                            ButtonTone::Quiet,
                            providers.len() > 1,
                        )
                        .min_w(px(0.0))
                        .max_w(px((width - 100.0).max(0.0)))
                        .overflow_hidden()
                        .dropdown_menu(move |mut menu, _, _| {
                            for provider in &providers {
                                let provider = provider.clone();
                                let entity = provider_entity.clone();
                                menu = menu.item(PopupMenuItem::new(provider.clone()).on_click(
                                    move |_, window, cx| {
                                        let _ = entity.update(cx, |app, cx| {
                                            app.choose_worker_picker_provider(
                                                provider.clone(),
                                                window,
                                                cx,
                                            )
                                        });
                                    },
                                ));
                            }
                            menu
                        }),
                    ),
            )
            .child(
                div()
                    .px(theme().space.sm)
                    .pb(theme().space.sm)
                    .child(Input::new(search)),
            )
            .child(result_list(
                "worker-model-results",
                models.len(),
                list_height,
                &self.workspace.runtime_picker.scroll,
                "No matching models.".to_owned(),
                {
                    let rows_entity = entity.clone();
                    move |row| {
                        let (index, choice) = models[row].clone();
                        let entity = rows_entity.clone();
                        let label = format!(
                            "{}{}",
                            if selected_index == Some(index) {
                                "✓ "
                            } else {
                                ""
                            },
                            choice.name
                        );
                        model_result_button(
                            ("worker-model", row),
                            label,
                            row == highlighted,
                            move |_, cx| {
                                let _ = entity.update(cx, |app, cx| {
                                    if let Some(worker) =
                                        app.workspace.runtime_picker.worker.as_mut()
                                    {
                                        worker.select(index);
                                    }
                                    cx.notify();
                                });
                            },
                        )
                        .into_any_element()
                    }
                },
            ))
            .when_some(
                selected.filter(|choice| !choice.efforts.is_empty()),
                |panel, choice| {
                    panel.child(worker_option_row(
                        "worker-effort",
                        "Effort",
                        &choice.efforts,
                        worker.effort.as_deref(),
                        entity.clone(),
                        |worker, value| worker.effort = value,
                    ))
                },
            )
            .when_some(
                selected.filter(|choice| !choice.service_tiers.is_empty()),
                |panel, choice| {
                    panel.child(worker_option_row(
                        "worker-service-tier",
                        "Service tier",
                        &choice.service_tiers,
                        worker.service_tier.as_deref(),
                        entity.clone(),
                        |worker, value| worker.service_tier = value,
                    ))
                },
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(theme().space.xs)
                    .p(theme().space.sm)
                    .border_t(theme().border)
                    .border_color(theme().colors.border)
                    .child({
                        let entity = entity.clone();
                        button(
                            "worker-use-once",
                            "Use once",
                            ButtonTone::Quiet,
                            selected.is_some(),
                            move |window, cx| {
                                let _ = entity.update(cx, |app, cx| {
                                    app.submit_worker_model_choice(false, window, cx)
                                });
                            },
                        )
                    })
                    .child({
                        let entity = entity.clone();
                        button(
                            "worker-save-model",
                            format!("Save for {profile}"),
                            ButtonTone::Accent,
                            selected.is_some(),
                            move |window, cx| {
                                let _ = entity.update(cx, |app, cx| {
                                    app.submit_worker_model_choice(true, window, cx)
                                });
                            },
                        )
                    }),
            )
            .into_any_element()
    }
}
