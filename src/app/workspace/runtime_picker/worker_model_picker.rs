use super::*;
use crate::protocol::{WorkerModelChoice, WorkerModelSelection};

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
            worker.selected = None;
            worker.effort = None;
        }
        self.reset_worker_model_search(window, cx);
    }

    fn choose_worker_picker_provider(
        &mut self,
        provider: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(worker) = self.workspace.runtime_picker.worker.as_mut() {
            worker.provider = provider;
            worker.selected = None;
            worker.effort = None;
        }
        self.reset_worker_model_search(window, cx);
    }

    fn reset_worker_model_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace.runtime_picker.highlighted = 0;
        self.workspace
            .runtime_picker
            .scroll
            .scroll_to_item(0, gpui::ScrollStrategy::Top);
        if let Some(search) = &self.workspace.runtime_picker.search {
            search.update(cx, |input, cx| input.set_value("", window, cx));
        }
        cx.notify();
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
                    worker.effort.as_ref().is_none_or(|effort| item.efforts.contains(effort)))
        );
        if !valid {
            return;
        }
        let Ok(value) = serde_json::to_string(&WorkerModelSelection {
            choice,
            effort: worker.effort.clone(),
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
            .filter(|(_, choice)| {
                format!("{} {}", choice.id, choice.name)
                    .to_lowercase()
                    .contains(&query)
            })
            .map(|(index, choice)| (index, choice.clone()))
            .collect::<Vec<(usize, WorkerModelChoice)>>();
        let selected = worker.selected.and_then(|index| choices.get(index));
        let (width, height, list_height) = layout::dimensions(
            f32::from(window.viewport_size().width),
            f32::from(window.viewport_size().height),
            models.len(),
        );
        let keyboard_models = models.clone();
        let selected_index = worker.selected;
        let highlighted = self.workspace.runtime_picker.highlighted;
        let selected_effort = worker.effort.clone();
        let focus = search.read(cx).focus_handle(cx);
        let keyboard_entity = entity.clone();
        let harness_entity = entity.clone();
        let provider_entity = entity.clone();
        div()
            .id("runtime-picker")
            .w(px(width))
            .max_h(px(height))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .bg(THEME.colors.panel)
            .border(THEME.border)
            .border_color(THEME.colors.border)
            .rounded(THEME.radius)
            .capture_key_down(move |event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified()
                    || !focus.contains_focused(window, cx)
                    || keyboard_models.is_empty()
                {
                    return;
                }
                let key = event.keystroke.key.as_str();
                if !matches!(key, "up" | "down" | "enter") {
                    return;
                }
                window.prevent_default();
                cx.stop_propagation();
                let _ = keyboard_entity.update(cx, |app, cx| {
                    let last = keyboard_models.len() - 1;
                    let current = app.workspace.runtime_picker.highlighted.min(last);
                    app.workspace.runtime_picker.highlighted = match key {
                        "up" => current.saturating_sub(1),
                        "down" => (current + 1).min(last),
                        _ => current,
                    };
                    if key == "enter"
                        && let Some(worker) = app.workspace.runtime_picker.worker.as_mut()
                    {
                        worker.selected = Some(keyboard_models[current].0);
                        worker.effort = None;
                    }
                    app.workspace.runtime_picker.scroll.scroll_to_item(
                        app.workspace.runtime_picker.highlighted,
                        gpui::ScrollStrategy::Center,
                    );
                    cx.notify();
                });
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(THEME.space.sm)
                    .p(THEME.space.sm)
                    .child(div().text_color(THEME.colors.muted).child("Harness"))
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
                    .gap(THEME.space.sm)
                    .px(THEME.space.sm)
                    .pb(THEME.space.sm)
                    .child(div().text_color(THEME.colors.muted).child("Provider"))
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
                    .px(THEME.space.sm)
                    .pb(THEME.space.sm)
                    .child(Input::new(search)),
            )
            .child(if models.is_empty() {
                div()
                    .p(THEME.space.sm)
                    .text_color(THEME.colors.muted)
                    .child("No matching models.")
                    .into_any_element()
            } else {
                let rows_entity = entity.clone();
                gpui::uniform_list("worker-model-results", models.len(), move |range, _, _| {
                    range
                        .map(|row| {
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
                                            worker.selected = Some(index);
                                            worker.effort = None;
                                        }
                                        cx.notify();
                                    });
                                },
                            )
                            .into_any_element()
                        })
                        .collect()
                })
                .h(px(list_height))
                .flex_none()
                .track_scroll(&self.workspace.runtime_picker.scroll)
                .into_any_element()
            })
            .when_some(
                selected.filter(|choice| !choice.efforts.is_empty()),
                |panel, choice| {
                    panel.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(THEME.space.sm)
                            .p(THEME.space.sm)
                            .border_t(THEME.border)
                            .border_color(THEME.colors.border)
                            .child(div().text_color(THEME.colors.muted).child("Effort"))
                            .child(
                                div().flex().flex_wrap().gap(THEME.space.xs).children(
                                    std::iter::once(None)
                                        .chain(choice.efforts.iter().cloned().map(Some))
                                        .enumerate()
                                        .map(|(index, effort)| {
                                            let entity = entity.clone();
                                            let chosen = selected_effort == effort;
                                            button(
                                                ("worker-effort", index),
                                                effort.clone().unwrap_or_else(|| "Default".into()),
                                                if chosen {
                                                    ButtonTone::Accent
                                                } else {
                                                    ButtonTone::Quiet
                                                },
                                                true,
                                                move |_, cx| {
                                                    let _ = entity.update(cx, |app, cx| {
                                                        if let Some(worker) = app
                                                            .workspace
                                                            .runtime_picker
                                                            .worker
                                                            .as_mut()
                                                        {
                                                            worker.effort = effort.clone();
                                                        }
                                                        cx.notify();
                                                    });
                                                },
                                            )
                                        }),
                                ),
                            ),
                    )
                },
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(THEME.space.xs)
                    .p(THEME.space.sm)
                    .border_t(THEME.border)
                    .border_color(THEME.colors.border)
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
