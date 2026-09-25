use super::*;
use crate::app::{
    ui::primitives::dropdown_content_button,
    workspace::worker_tasks::{
        WorkerModelEdit, WorkerProfileEdit, WorkerRouteChoice, WorkerRouteTarget, model_efforts,
    },
};
use gpui_component::{
    Disableable as _,
    menu::{DropdownMenu as _, PopupMenuItem},
};

pub(super) fn render(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let editor = &app.workspace.worker_profile_editor;
    if !editor.loaded {
        return div()
            .flex()
            .flex_col()
            .gap(theme().space.sm)
            .child("Worker profiles could not be loaded.")
            .when_some(editor.error.as_ref(), |view, error| {
                view.child(
                    div()
                        .text_color(theme().colors.danger)
                        .child(error.to_owned()),
                )
            })
            .child(button(
                "retry-worker-profiles",
                "Retry loading profiles",
                ButtonTone::Quiet,
                true,
                move |_, cx| {
                    let _ = entity.update(cx, |this, cx| this.retry_worker_profile_settings(cx));
                },
            ))
            .into_any_element();
    }
    let editing = editor.edit.is_some();
    let reload = entity.clone();
    div()
        .flex()
        .flex_col()
        .gap(theme().space.md)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(theme().space.md)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(theme().space.xs)
                        .child(
                            div()
                                .text_size(theme().type_scale.reading)
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("Worker profiles"),
                        )
                        .child(
                            div()
                                .text_size(theme().type_scale.body_small)
                                .text_color(theme().colors.muted)
                                .child(
                                    "Choose one model and an active worker limit for each profile. An empty profile asks when first used.",
                                ),
                        ),
                )
                .child(button(
                    "worker-reload-choices",
                    "Reload choices",
                    ButtonTone::Quiet,
                    !editing,
                    move |_, cx| {
                        let _ = reload.update(cx, |this, cx| this.reload_worker_choices(cx));
                    },
                )),
        )
        .child(
            div()
                .flex()
                .gap(theme().space.md)
                .child(profile_rail(app, entity.clone()))
                .child(div().w(theme().size(1.0)).bg(theme().colors.surface).flex_none())
                .child(profile_detail(app, entity)),
        )
        .into_any_element()
}

fn profile_rail(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let editor = &app.workspace.worker_profile_editor;
    let editing = editor.edit.is_some();
    let add = entity.clone();
    let inherit = entity.clone();
    let mut rail = div()
        .flex()
        .flex_col()
        .gap(theme().space.xs)
        .w(theme().size(132.0))
        .flex_none()
        .child(
            button(
                "worker-profile-inherit",
                "inherit",
                ButtonTone::Quiet,
                !editing,
                move |_, cx| {
                    let _ = inherit.update(cx, |this, cx| {
                        this.workspace.worker_profile_editor.inherit_selected = true;
                        this.workspace.worker_profile_editor.error = None;
                        cx.notify();
                    });
                },
            )
            .w_full()
            .justify_start()
            .toggled(editor.inherit_selected),
        );
    for (index, profile) in editor.profiles.iter().enumerate() {
        let entity = entity.clone();
        rail = rail.child(
            button(
                ("worker-profile", index),
                if profile.enabled {
                    profile.name.clone()
                } else {
                    format!("{} (off)", profile.name)
                },
                ButtonTone::Quiet,
                !editing,
                move |_, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.workspace.worker_profile_editor.selected = index;
                        this.workspace.worker_profile_editor.inherit_selected = false;
                        this.workspace.worker_profile_editor.selected_model = 0;
                        this.workspace.worker_profile_editor.error = None;
                        cx.notify();
                    });
                },
            )
            .w_full()
            .justify_start()
            .toggled(!editor.inherit_selected && index == editor.selected),
        );
    }
    rail = rail.child(
        button(
            "worker-profile-add",
            "+ Add profile",
            ButtonTone::Quiet,
            !editing && !editor.has_draft(),
            move |window, cx| {
                let _ = add.update(cx, |this, cx| this.edit_worker_profile(None, window, cx));
            },
        )
        .w_full()
        .justify_start(),
    );

    rail.into_any_element()
}

fn profile_detail(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let editor = &app.workspace.worker_profile_editor;
    let editing = editor.edit.is_some();
    let mut detail = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(theme().space.sm);
    if let Some(edit @ (WorkerProfileEdit::Name { .. } | WorkerProfileEdit::Limit { .. })) =
        &editor.edit
    {
        detail = detail.child(edit_form(edit, entity.clone()));
    } else if editor.inherit_selected {
        detail = detail
            .child(
                div()
                    .text_size(theme().type_scale.body)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Same as caller"),
            )
            .child(
                div()
                    .text_size(theme().type_scale.body_small)
                    .text_color(theme().colors.muted)
                    .child("Uses the caller's harness, provider, model, and effort."),
            );
        let limit = entity.clone();
        let toggle = entity.clone();
        detail = detail.child(
            div()
                .flex()
                .gap(theme().space.sm)
                .child(button(
                    "inherit-limit",
                    format!("Limit: {} active", editor.inherit_limit),
                    ButtonTone::Quiet,
                    !editing,
                    move |window, cx| {
                        let _ =
                            limit.update(cx, |this, cx| this.edit_worker_limit(None, window, cx));
                    },
                ))
                .child(button(
                    "inherit-enabled",
                    if editor.inherit_enabled {
                        "Disable"
                    } else {
                        "Enable"
                    },
                    ButtonTone::Quiet,
                    !editing,
                    move |_, cx| {
                        let _ = toggle.update(cx, |this, cx| this.toggle_worker_profile(None, cx));
                    },
                )),
        );
    } else if let Some(profile) = editor.profiles.get(editor.selected) {
        let rename = entity.clone();
        let delete = entity.clone();
        let toggle = entity.clone();
        let selected = editor.selected;
        let enabled = profile.enabled;
        let built_in = matches!(
            profile.name.as_str(),
            "smartest" | "smart" | "standard" | "light"
        );
        detail = detail.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(theme().type_scale.body)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(profile.name.clone()),
                )
                .child(
                    actions_button("worker-profile-actions", "Profile actions", !editing)
                        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
                            let rename = rename.clone();
                            let delete = delete.clone();
                            let toggle = toggle.clone();
                            let menu = menu
                                .item(PopupMenuItem::new("Edit profile…").on_click(
                                    move |_, window, cx| {
                                        let _ = rename.update(cx, |this, cx| {
                                            this.edit_worker_profile(Some(selected), window, cx)
                                        });
                                    },
                                ))
                                .item(
                                    PopupMenuItem::new(if enabled {
                                        "Disable profile"
                                    } else {
                                        "Enable profile"
                                    })
                                    .on_click(
                                        move |_, _, cx| {
                                            let _ = toggle.update(cx, |this, cx| {
                                                this.toggle_worker_profile(Some(selected), cx)
                                            });
                                        },
                                    ),
                                );
                            if built_in {
                                menu
                            } else {
                                menu.item(PopupMenuItem::new("Delete profile").on_click(
                                    move |_, _, cx| {
                                        let _ = delete
                                            .update(cx, |this, cx| this.delete_worker_profile(cx));
                                    },
                                ))
                            }
                        }),
                ),
        );
        detail = detail.child(
            div()
                .text_size(theme().type_scale.body_small)
                .text_color(theme().colors.muted)
                .child(profile.description.clone()),
        );
        let limit = entity.clone();
        detail = detail.child(button(
            "worker-profile-limit",
            format!("Limit: {} active", profile.limit),
            ButtonTone::Quiet,
            !editing,
            move |window, cx| {
                let _ = limit.update(cx, |this, cx| {
                    this.edit_worker_limit(Some(selected), window, cx)
                });
            },
        ));
        if profile.models.is_empty() {
            let add = entity.clone();
            let target = WorkerRouteTarget {
                profile: selected,
                model: 0,
            };
            detail = detail
                .child(
                    div()
                        .text_size(theme().type_scale.body_small)
                        .text_color(theme().colors.muted)
                        .child("No model selected. The first worker request will ask you to choose one."),
                )
                .child(button(
                    "worker-model-add-empty",
                    "Choose model",
                    ButtonTone::Quiet,
                    !editing,
                    move |_, cx| {
                        let _ = add.update(cx, |this, cx| {
                            this.edit_worker_models(target, WorkerModelEdit::Add, cx)
                        });
                    },
                ));
        } else if let Some(model) = profile.models.first() {
            let target = WorkerRouteTarget {
                profile: selected,
                model: 0,
            };
            let clear = entity.clone();
            detail = detail.child(button(
                "worker-model-clear",
                "Clear model",
                ButtonTone::Quiet,
                !editing,
                move |_, cx| {
                    let _ = clear.update(cx, |this, cx| {
                        this.edit_worker_models(target, WorkerModelEdit::Remove, cx)
                    });
                },
            ));
            detail = detail.child(route(app, entity.clone(), target));
            if model.validate().is_err() {
                detail = detail.child(div().text_size(theme().type_scale.caption)
                    .text_color(theme().colors.muted)
                    .child("Not saved yet. Choose a provider and model; saved settings are unchanged."));
            }
            if let Some(edit @ WorkerProfileEdit::Custom { target: edited, .. }) = &editor.edit
                && *edited == target
            {
                detail = detail.child(edit_form(edit, entity.clone()));
            }
        }
    } else {
        detail = detail.child(
            div()
                .py(theme().space.md)
                .text_color(theme().colors.muted)
                .child("Add a custom profile, or use Same as caller."),
        );
    }
    if let Some(error) = &editor.error {
        detail = detail.child(feedback(
            "worker-profile-error",
            error.clone(),
            FeedbackTone::Error,
        ));
    }
    detail.into_any_element()
}

fn route(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
    target: WorkerRouteTarget,
) -> AnyElement {
    let editor = &app.workspace.worker_profile_editor;
    let profile = &editor.profiles[target.profile];
    let route = &profile.models[target.model];
    let catalog = editor.catalog(route.harness, &app.project.path);
    let enabled = editor.edit.is_none();
    let harnesses = crate::agents::backend_statuses()
        .into_iter()
        .map(|backend| {
            (
                if backend.available {
                    backend.name
                } else {
                    format!("{} (not installed)", backend.name)
                },
                backend.id == route.harness,
                WorkerRouteChoice::Harness(backend.id),
            )
        })
        .collect();
    let providers = catalog
        .models
        .iter()
        .map(|model| model.provider.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|provider| {
            (
                provider.clone(),
                provider == route.provider,
                WorkerRouteChoice::Provider(provider),
            )
        })
        .collect();
    let models = catalog
        .models
        .iter()
        .filter(|model| model.provider == route.provider)
        .map(|model| {
            (
                model_label(model),
                model.id == route.model,
                WorkerRouteChoice::Model {
                    provider: model.provider.clone(),
                    id: model.id.clone(),
                },
            )
        })
        .collect();
    let selected_model = catalog
        .models
        .iter()
        .find(|model| model.provider == route.provider && model.id == route.model);
    let model_label = selected_model
        .map(model_label)
        .unwrap_or_else(|| selected(&route.model, "Select model"));
    let efforts = std::iter::once(String::new())
        .chain(model_efforts(&catalog, selected_model).iter().cloned())
        .map(|effort| {
            (
                selected(&effort, "Default"),
                route.effort.as_deref().unwrap_or_default() == effort,
                WorkerRouteChoice::Effort(effort),
            )
        })
        .collect();
    let tiers = std::iter::once(String::new())
        .chain(
            selected_model
                .into_iter()
                .flat_map(|model| model.service_tiers.iter().cloned()),
        )
        .map(|tier| {
            (
                selected(&tier, "Default"),
                route.service_tier.as_deref().unwrap_or_default() == tier,
                WorkerRouteChoice::ServiceTier(tier),
            )
        })
        .collect();
    let custom = entity.clone();
    let label = format!("Model {}", target.model + 1);
    let explanation =
        "Move a model up to prefer it. Missing harnesses and unlisted models are skipped.";
    let mut row = div()
        .flex()
        .flex_col()
        .gap(theme().space.sm)
        .py(theme().space.sm)
        .border_t_1()
        .border_color(theme().colors.surface)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(theme().space.sm)
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(theme().space.xs)
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme().colors.text)
                                .child(label),
                        )
                        .child(
                            div()
                                .text_size(theme().type_scale.caption)
                                .text_color(theme().colors.muted)
                                .child(explanation),
                        ),
                )
                .child(
                    actions_button(
                        ("worker-route-actions", target.model),
                        "Model settings",
                        enabled,
                    )
                    .dropdown_menu_with_anchor(
                        gpui::Anchor::TopRight,
                        move |menu, _, _| {
                            let custom = custom.clone();
                            menu.item(PopupMenuItem::new("Enter custom IDs…").on_click(
                                move |_, window, cx| {
                                    let _ = custom.update(cx, |this, cx| {
                                        this.edit_worker_custom_route(target, window, cx)
                                    });
                                },
                            ))
                        },
                    ),
                ),
        )
        .child(
            div().flex().gap(theme().space.sm).children(
                [
                    (
                        "worker-harness",
                        crate::agents::backend_display_name(route.harness),
                        harnesses,
                        enabled,
                    ),
                    (
                        "worker-provider",
                        selected(
                            &route.provider,
                            if catalog.models.is_empty() {
                                "No providers"
                            } else {
                                "Select provider"
                            },
                        ),
                        providers,
                        enabled,
                    ),
                    (
                        "worker-model",
                        model_label,
                        models,
                        enabled && !route.provider.is_empty(),
                    ),
                    (
                        "worker-effort",
                        selected(route.effort.as_deref().unwrap_or_default(), "Default"),
                        efforts,
                        enabled && selected_model.is_some(),
                    ),
                ]
                .into_iter()
                .map(|(id, label, choices, enabled)| {
                    route_menu(id, label, choices, target, enabled, entity.clone())
                }),
            ),
        );
    if selected_model.is_some_and(|model| !model.service_tiers.is_empty())
        || route.service_tier.is_some()
    {
        row = row.child(route_menu(
            "worker-service-tier",
            selected(route.service_tier.as_deref().unwrap_or_default(), "Default"),
            tiers,
            target,
            enabled && selected_model.is_some(),
            entity.clone(),
        ));
    }
    if catalog.models.is_empty() {
        row = row.child(div().text_size(theme().type_scale.caption).text_color(theme().colors.subtle)
            .child("No catalog yet. Open a session with this harness, then reload choices, or use custom IDs."));
    } else if !route.model.is_empty() && selected_model.is_none() {
        row = row.child(
            div()
                .text_size(theme().type_scale.caption)
                .text_color(theme().colors.subtle)
                .child(
                    "This model is not in the saved catalog. Worker creation will ask you to choose again. Reload choices or choose a listed model.",
                ),
        );
    }
    row.into_any_element()
}

fn route_menu(
    id: &'static str,
    label: String,
    choices: Vec<(String, bool, WorkerRouteChoice)>,
    target: WorkerRouteTarget,
    enabled: bool,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let field = match id {
        "worker-harness" => "Harness",
        "worker-provider" => "Provider",
        "worker-model" => "Model",
        "worker-service-tier" => "Service tier",
        _ => "Effort",
    };
    div()
        .flex_1()
        .when(id == "worker-model", |field| field.flex_grow(2.0))
        .min_w_0()
        .flex()
        .flex_col()
        .gap(theme().space.xs)
        .child(
            div()
                .text_size(theme().type_scale.caption)
                .text_color(theme().colors.muted)
                .child(field),
        )
        .child(
            dropdown_content_button(
                (id, target.model),
                format!("{}: {label}", id.trim_start_matches("worker-")),
                div().flex_1().min_w_0().truncate().child(label),
                ButtonTone::Neutral,
                enabled && !choices.is_empty(),
            )
            .w_full()
            .dropdown_menu_with_anchor(gpui::Anchor::TopLeft, move |menu, _, _| {
                choices.iter().fold(
                    menu.min_w(theme().size(180.0))
                        .max_h(theme().size(320.0))
                        .scrollable(true),
                    |menu, (label, checked, choice)| {
                        let entity = entity.clone();
                        let choice = choice.clone();
                        menu.item(
                            PopupMenuItem::new(label.clone())
                                .checked(*checked)
                                .on_click(move |_, _, cx| {
                                    let _ = entity.update(cx, |this, cx| {
                                        this.select_worker_route(target, choice.clone(), cx)
                                    });
                                }),
                        )
                    },
                )
            }),
        )
        .into_any_element()
}

fn edit_form(edit: &WorkerProfileEdit, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let mut form = div()
        .flex()
        .flex_col()
        .gap(theme().space.sm)
        .p(theme().space.sm)
        .bg(theme().colors.surface)
        .rounded(theme().radius);
    match edit {
        WorkerProfileEdit::Name {
            profile,
            input,
            description,
            limit,
        } => {
            form = form
                .child(div().child(if profile.is_some() {
                    "Edit profile"
                } else {
                    "New profile"
                }))
                .when(profile.is_none(), |form| form.child(Input::new(input)))
                .child(div().child("When to use"))
                .child(Input::new(description))
                .child(div().child("Maximum active workers"))
                .child(Input::new(limit))
                .child(
                    div()
                        .text_size(theme().type_scale.caption)
                        .text_color(theme().colors.muted)
                        .child("Use letters, numbers, '-' or '_'."),
                );
        }
        WorkerProfileEdit::Limit { input, .. } => {
            form = form
                .child(div().child("Maximum active workers"))
                .child(Input::new(input));
        }
        WorkerProfileEdit::Custom { inputs, .. } => {
            form = form.child(div().child("Custom IDs"))
                .child(div().text_size(theme().type_scale.caption).text_color(theme().colors.muted).child("Use exact IDs for models not listed by the harness. Leave effort and service tier blank for their defaults."))
                .child(div().flex().gap(theme().space.sm).children(["Provider ID", "Model ID", "Effort", "Service tier"].into_iter().zip(inputs).map(|(label, input)| {
                    div().flex_1().min_w_0().flex().flex_col().gap(theme().space.xs)
                        .child(div().text_size(theme().type_scale.caption).text_color(theme().colors.muted).child(label))
                        .child(Input::new(input))
                })));
        }
    };
    form.child(
        div()
            .flex()
            .justify_end()
            .gap(theme().space.sm)
            .child(button(
                "finish-worker-edit",
                "Done",
                ButtonTone::Neutral,
                true,
                move |window, cx| {
                    let _ =
                        entity.update(cx, |this, cx| this.finish_worker_profile_edit(window, cx));
                },
            )),
    )
    .into_any_element()
}

fn actions_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
    enabled: bool,
) -> Button {
    let label = label.into();
    Button::new(id)
        .label("…")
        .accessibility_label(label.clone())
        .tooltip(label)
        .with_size(Size::Small)
        .ghost()
        .disabled(!enabled)
}

fn selected(value: &str, placeholder: &str) -> String {
    if value.is_empty() {
        placeholder.into()
    } else {
        value.into()
    }
}

fn model_label(model: &crate::protocol::Model) -> String {
    selected(&model.name, &model.id)
}
