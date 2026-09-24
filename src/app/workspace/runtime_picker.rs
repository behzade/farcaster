use super::*;
mod layout;
mod worker_model_picker;
use crate::app::ui::{
    primitives::{ButtonTone, button, dropdown_button},
    theme::THEME,
};
use gpui::{
    InteractiveElement as _, IntoElement as _, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
};

pub(in crate::app) struct WorkerModelPicker {
    id: String,
    harness: crate::agents::Backend,
    provider: String,
    selected: Option<usize>,
    effort: Option<String>,
    service_tier: Option<String>,
}

#[derive(Default)]
pub(in crate::app) struct RuntimePickerState {
    pub(in crate::app) open: bool,
    pub(in crate::app) worker: Option<WorkerModelPicker>,
    provider: Option<String>,
    search: Option<Entity<InputState>>,
    subscription: Option<Subscription>,
    highlighted: usize,
    scroll: gpui::UniformListScrollHandle,
}

fn model_result_button(
    id: impl Into<gpui::ElementId>,
    label: String,
    highlighted: bool,
    on_press: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl gpui::IntoElement {
    button(id, "", ButtonTone::Quiet, true, on_press)
        .accessibility_label(label.clone())
        .tooltip(label.clone())
        .child(div().w_full().min_w(px(0.0)).truncate().child(label))
        .w_full()
        .min_w(px(0.0))
        .overflow_hidden()
        .h(px(32.0))
        .justify_start()
        .when(highlighted, |row| row.bg(THEME.colors.surface))
}

impl FarcasterApp {
    pub(in crate::app) fn close_worker_picker_for_surface_switch(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.runtime_picker.open && self.workspace.runtime_picker.worker.is_some() {
            self.set_runtime_picker_open(false, window, cx);
        }
    }

    pub(in crate::app) fn set_worker_model_picker_open(
        &mut self,
        open: bool,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if open {
            let Some(crate::protocol::ExtensionUiRequest::WorkerModel {
                id: active,
                choices,
                ..
            }) = self.extensions.active.dialog.as_ref()
            else {
                return;
            };
            if active != id || choices.is_empty() {
                return;
            }
            let first = &choices[0];
            self.workspace.runtime_picker.worker = Some(WorkerModelPicker {
                id: id.to_owned(),
                harness: first.harness,
                provider: first.provider.clone(),
                selected: None,
                effort: None,
                service_tier: None,
            });
        }
        self.set_runtime_picker_open(open, window, cx);
    }

    pub(in crate::app) fn close_stale_worker_model_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(worker) = self.workspace.runtime_picker.worker.as_ref()
            && self
                .extensions
                .active
                .dialog
                .as_ref()
                .and_then(crate::protocol::ExtensionUiRequest::dialog_id)
                != Some(worker.id.as_str())
        {
            self.set_runtime_picker_open(false, window, cx);
        }
    }

    pub(in crate::app) fn set_runtime_picker_open(
        &mut self,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.runtime_picker.open = open;
        if open {
            if self.workspace.runtime_picker.worker.is_none()
                && let Some(harness) = self.snapshot.harness
            {
                self.send(
                    RuntimeCommand::LoadConfiguration {
                        harness,
                        project: self.snapshot.project.clone(),
                    },
                    cx,
                );
            }
            self.workspace.runtime_picker.highlighted = 0;
            self.workspace.runtime_picker.scroll = gpui::UniformListScrollHandle::new();
            self.workspace.runtime_picker.provider = self
                .workspace
                .runtime_picker
                .worker
                .as_ref()
                .map(|worker| worker.provider.clone())
                .or_else(|| self.snapshot.session_identity().provider.map(str::to_owned));
            if self.workspace.runtime_picker.worker.is_none()
                && let Some(selected) = self.snapshot.session_identity().model
            {
                self.workspace.runtime_picker.highlighted = self
                    .snapshot
                    .models
                    .iter()
                    .filter(|model| model.provider == selected.provider)
                    .position(|model| model.id == selected.id)
                    .unwrap_or(0);
                self.workspace.runtime_picker.scroll.scroll_to_item(
                    self.workspace.runtime_picker.highlighted,
                    gpui::ScrollStrategy::Center,
                );
            }
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search models…"));
            self.workspace.runtime_picker.subscription =
                Some(cx.subscribe(&input, |app, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        app.workspace.runtime_picker.highlighted = 0;
                        app.workspace
                            .runtime_picker
                            .scroll
                            .scroll_to_item(0, gpui::ScrollStrategy::Top);
                    }
                    cx.notify();
                }));
            let focus = input.read(cx).focus_handle(cx);
            self.workspace.runtime_picker.search = Some(input);
            cx.defer_in(window, move |_, window, cx| focus.focus(window, cx));
        } else {
            if self
                .workspace
                .runtime_picker
                .search
                .as_ref()
                .is_some_and(|input| input.read(cx).focus_handle(cx).contains_focused(window, cx))
            {
                if self.workspace.runtime_picker.worker.is_some()
                    && self.extensions.active.dialog.is_some()
                {
                    self.extensions.dialog_focus.focus(window, cx);
                } else {
                    self.composer.focus.focus(window, cx);
                }
            }
            self.workspace.runtime_picker.subscription = None;
            self.workspace.runtime_picker.search = None;
            self.workspace.runtime_picker.worker = None;
        }
        cx.notify();
    }

    pub(in crate::app) fn render_runtime_picker(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        if self.workspace.runtime_picker.worker.is_some() {
            return self.render_worker_model_picker(window, cx);
        }
        let Some(search) = self.workspace.runtime_picker.search.as_ref() else {
            return div().into_any_element();
        };
        let entity = cx.entity().downgrade();
        let mut providers = self
            .snapshot
            .models
            .iter()
            .map(|model| model.provider.clone())
            .collect::<Vec<_>>();
        providers.sort();
        providers.dedup();
        let provider = self
            .workspace
            .runtime_picker
            .provider
            .as_ref()
            .filter(|provider| providers.contains(provider))
            .or_else(|| providers.first())
            .cloned();
        let query = search.read(cx).value().trim().to_lowercase();
        let identity = self.snapshot.session_identity();
        let models = self
            .snapshot
            .models
            .iter()
            .filter(|model| Some(&model.provider) == provider.as_ref())
            .filter(|model| {
                format!("{} {}", model.id, model.name)
                    .to_lowercase()
                    .contains(&query)
            })
            .cloned()
            .collect::<Vec<_>>();
        let selected = identity
            .model
            .map(|model| self.snapshot.catalog_model(model));
        let feedback = if self.snapshot.models.is_empty() {
            match &self.snapshot.configuration_status {
                crate::runtime::ConfigurationStatus::Loading => "Loading models…".to_owned(),
                crate::runtime::ConfigurationStatus::Failed(error) => {
                    format!("Models unavailable: {error}")
                }
                crate::runtime::ConfigurationStatus::Loaded => {
                    "No models advertised by this harness.".to_owned()
                }
            }
        } else {
            "No matching models.".to_owned()
        };
        let levels = selected
            .filter(|model| model.reasoning && Some(&model.provider) == provider.as_ref())
            .map(|model| self.snapshot.effort_choices(model))
            .unwrap_or_default();
        let service_tiers = self.snapshot.available_service_tiers();
        let selected_tier = self.snapshot.selected_service_tier();
        let provider_entity = entity.clone();
        let keyboard_entity = entity.clone();
        let keyboard_models = models.clone();
        let search_focus = search.read(cx).focus_handle(cx);
        let selected_id = selected.map(|model| (model.provider.clone(), model.id.clone()));
        let highlighted = self.workspace.runtime_picker.highlighted;
        // Only the results pane grows. The outer scroll is a fallback for short windows.
        let (width, height, list_height) = layout::dimensions(
            f32::from(window.viewport_size().width),
            f32::from(window.viewport_size().height),
            models.len(),
        );
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
                    || !search_focus.contains_focused(window, cx)
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
                    if key == "enter" {
                        app.select_model_from_ui(&keyboard_models[current], window, cx);
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
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(THEME.space.sm)
                    .p(THEME.space.sm)
                    .child(div().text_color(THEME.colors.muted).child("Provider"))
                    .child(
                        dropdown_button(
                            "runtime-provider",
                            provider.unwrap_or_else(|| "Provider".into()),
                            ButtonTone::Quiet,
                            providers.len() > 1,
                        )
                        .min_w(px(0.0))
                        .max_w(px((width - 100.0).max(0.0)))
                        .overflow_hidden()
                        .dropdown_menu(move |mut menu, _, _| {
                            for provider in &providers {
                                let entity = provider_entity.clone();
                                let provider = provider.clone();
                                menu = menu.item(PopupMenuItem::new(provider.clone()).on_click(
                                    move |_, window, cx| {
                                        let _ = entity.update(cx, |app, cx| {
                                            app.workspace.runtime_picker.provider =
                                                Some(provider.clone());
                                            app.workspace.runtime_picker.highlighted = 0;
                                            app.workspace
                                                .runtime_picker
                                                .scroll
                                                .scroll_to_item(0, gpui::ScrollStrategy::Top);
                                            if let Some(search) =
                                                &app.workspace.runtime_picker.search
                                            {
                                                search.update(cx, |input, cx| {
                                                    input.set_value("", window, cx)
                                                });
                                            }
                                            cx.notify();
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
                    .flex_none()
                    .px(THEME.space.sm)
                    .pb(THEME.space.sm)
                    .child(Input::new(search)),
            )
            .child(if models.is_empty() {
                div()
                    .p(THEME.space.sm)
                    .text_color(THEME.colors.muted)
                    .child(feedback)
                    .into_any_element()
            } else {
                let rows_entity = entity.clone();
                gpui::uniform_list("runtime-model-results", models.len(), move |range, _, _| {
                    range
                        .map(|index| {
                            let model = models[index].clone();
                            let current = selected_id.as_ref().is_some_and(|(provider, id)| {
                                *id == model.id && *provider == model.provider
                            });
                            let entity = rows_entity.clone();
                            let label =
                                format!("{}{}", if current { "✓ " } else { "" }, model.name);
                            model_result_button(
                                ("runtime-model", index),
                                label,
                                index == highlighted,
                                move |window, cx| {
                                    let _ = entity.update(cx, |app, cx| {
                                        app.select_model_from_ui(&model, window, cx)
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
            .when(levels.iter().any(Option::is_some), |panel| {
                panel.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(THEME.space.sm)
                        .p(THEME.space.sm)
                        .border_t(THEME.border)
                        .border_color(THEME.colors.border)
                        .child(
                            div()
                                .text_color(THEME.colors.muted)
                                .child(crate::agents::effort_label(self.snapshot.harness)),
                        )
                        .child(div().flex().flex_wrap().gap(THEME.space.xs).children(
                            levels.iter().cloned().enumerate().map(|(index, level)| {
                                let entity = entity.clone();
                                let current = identity.effort == level.as_deref();
                                button(
                                    ("runtime-effort", index),
                                    level.clone().unwrap_or_else(|| "Default".into()),
                                    if current {
                                        ButtonTone::Accent
                                    } else {
                                        ButtonTone::Quiet
                                    },
                                    true,
                                    move |_, cx| {
                                        let _ = entity.update(cx, |app, cx| {
                                            app.set_thinking_level(level.clone(), cx)
                                        });
                                    },
                                )
                            }),
                        )),
                )
            })
            .when(!service_tiers.is_empty(), |panel| {
                panel.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(THEME.space.sm)
                        .p(THEME.space.sm)
                        .border_t(THEME.border)
                        .border_color(THEME.colors.border)
                        .child(div().text_color(THEME.colors.muted).child("Service tier"))
                        .child(div().flex().flex_wrap().gap(THEME.space.xs).children(
                            service_tiers.iter().enumerate().map(|(index, tier)| {
                                let entity = entity.clone();
                                let tier = tier.clone();
                                button(
                                    ("runtime-service-tier", index),
                                    tier.clone(),
                                    if selected_tier == Some(tier.as_str()) {
                                        ButtonTone::Accent
                                    } else {
                                        ButtonTone::Quiet
                                    },
                                    true,
                                    move |_, cx| {
                                        let _ = entity.update(cx, |app, cx| {
                                            app.set_service_tier(tier.clone(), cx)
                                        });
                                    },
                                )
                            }),
                        )),
                )
            })
            .into_any_element()
    }
}
