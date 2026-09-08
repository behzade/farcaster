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
    let editor = &app.worker_profile_editor;
    let editing = editor.edit.is_some();
    let reload = entity.clone();
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.md)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(THEME.space.md)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.xs)
                        .child(
                            div()
                                .text_size(THEME.type_scale.reading)
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("Worker profiles"),
                        )
                        .child(
                            div()
                                .text_size(THEME.type_scale.body_small)
                                .text_color(THEME.colors.muted)
                                .child("Each profile uses the first available model in its list."),
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
                .gap(THEME.space.md)
                .child(profile_rail(app, entity.clone()))
                .child(div().w(gpui::px(1.0)).bg(THEME.colors.surface).flex_none())
                .child(profile_detail(app, entity)),
        )
        .into_any_element()
}

fn profile_rail(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let editor = &app.worker_profile_editor;
    let editing = editor.edit.is_some();
    let add = entity.clone();
    let mut rail = div()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .w(gpui::px(132.0))
        .flex_none();
    for (index, profile) in editor.profiles.iter().enumerate() {
        let entity = entity.clone();
        rail = rail.child(
            button(
                ("worker-profile", index),
                profile.name.clone(),
                ButtonTone::Quiet,
                !editing,
                move |_, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.worker_profile_editor.selected = index;
                        this.worker_profile_editor.selected_model = 0;
                        this.worker_profile_editor.error = None;
                        cx.notify();
                    });
                },
            )
            .w_full()
            .justify_start()
            .toggled(index == editor.selected),
        );
    }
    rail = rail.child(
        button(
            "worker-profile-add",
            "+ Add profile",
            ButtonTone::Quiet,
            !editing,
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
    let editor = &app.worker_profile_editor;
    let editing = editor.edit.is_some();
    let mut detail = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(THEME.space.sm);
    if let Some(edit @ WorkerProfileEdit::Name { .. }) = &editor.edit {
        detail = detail.child(edit_form(edit, entity.clone()));
    } else if let Some(profile) = editor.profiles.get(editor.selected) {
        let rename = entity.clone();
        let delete = entity.clone();
        let selected = editor.selected;
        detail = detail.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(THEME.type_scale.body)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(profile.name.clone()),
                )
                .child(
                    actions_button("worker-profile-actions", "Profile actions", !editing)
                        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
                            let rename = rename.clone();
                            let delete = delete.clone();
                            menu.item(PopupMenuItem::new("Edit profile…").on_click(
                                move |_, window, cx| {
                                    let _ = rename.update(cx, |this, cx| {
                                        this.edit_worker_profile(Some(selected), window, cx)
                                    });
                                },
                            ))
                            .item(
                                PopupMenuItem::new("Delete profile").on_click(move |_, _, cx| {
                                    let _ = delete
                                        .update(cx, |this, cx| this.delete_worker_profile(cx));
                                }),
                            )
                        }),
                ),
        );
        detail = detail.child(
            div()
                .text_size(THEME.type_scale.body_small)
                .text_color(THEME.colors.muted)
                .child(profile.description.clone()),
        );
        for (index, model) in profile.models.iter().enumerate() {
            let select = entity.clone();
            let catalog = editor.catalog(&model.harness, &app.project);
            let name = catalog
                .models
                .iter()
                .find(|candidate| {
                    candidate.provider == model.provider && candidate.id == model.model
                })
                .map(model_label)
                .unwrap_or_else(|| model.model.clone());
            let label = format!(
                "{}. {} · {}",
                index + 1,
                crate::agents::backend_display_name(&model.harness),
                name
            );
            detail = detail.child(
                Button::new(("worker-model-choice", index))
                    .accessibility_label(label.clone())
                    .tooltip(label.clone())
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .with_size(Size::Small)
                    .ghost()
                    .disabled(editing)
                    .on_click(move |_, _, cx| {
                        let _ = select.update(cx, |this, cx| {
                            this.worker_profile_editor.selected_model = index;
                            cx.notify();
                        });
                    })
                    .w_full()
                    .min_w_0()
                    .justify_start()
                    .toggled(index == editor.selected_model),
            );
        }
        if let Some(model) = profile.models.get(editor.selected_model) {
            let target = WorkerRouteTarget {
                profile: selected,
                model: editor.selected_model,
            };
            detail = detail.child(
                div().flex().gap(THEME.space.sm).children(
                    [
                        (
                            "worker-model-add",
                            "+ Add model",
                            WorkerModelEdit::Add,
                            true,
                        ),
                        (
                            "worker-model-up",
                            "Move up",
                            WorkerModelEdit::MoveUp,
                            target.model > 0,
                        ),
                        (
                            "worker-model-down",
                            "Move down",
                            WorkerModelEdit::MoveDown,
                            target.model + 1 < profile.models.len(),
                        ),
                        (
                            "worker-model-remove",
                            "Remove model",
                            WorkerModelEdit::Remove,
                            profile.models.len() > 1,
                        ),
                    ]
                    .into_iter()
                    .map(|(id, label, edit, enabled)| {
                        let entity = entity.clone();
                        button(
                            id,
                            label,
                            ButtonTone::Quiet,
                            enabled && !editing,
                            move |_, cx| {
                                let _ = entity.update(cx, |this, cx| {
                                    this.edit_worker_models(target, edit, cx)
                                });
                            },
                        )
                    }),
                ),
            );
            detail = detail.child(route(app, entity.clone(), target));
            if model.validate().is_err() {
                detail = detail.child(div().text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.muted)
                    .child("Not saved yet. Choose a provider and model; the previous route is still in use."));
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
                .py(THEME.space.md)
                .text_color(THEME.colors.muted)
                .child(
                    "Add a profile to configure a worker. With no profiles, new workers cannot start.",
                ),
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
    let editor = &app.worker_profile_editor;
    let profile = &editor.profiles[target.profile];
    let route = &profile.models[target.model];
    let catalog = editor.catalog(&route.harness, &app.project);
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
    let custom = entity.clone();
    let label = format!("Model {}", target.model + 1);
    let explanation =
        "Move a model up to prefer it. Missing harnesses and unlisted models are skipped.";
    let mut row = div()
        .flex()
        .flex_col()
        .gap(THEME.space.sm)
        .py(THEME.space.sm)
        .border_t_1()
        .border_color(THEME.colors.surface)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(THEME.space.sm)
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.xs)
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(THEME.colors.text)
                                .child(label),
                        )
                        .child(
                            div()
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.muted)
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
            div().flex().gap(THEME.space.sm).children(
                [
                    (
                        "worker-harness",
                        crate::agents::backend_display_name(&route.harness),
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
    if catalog.models.is_empty() {
        row = row.child(div().text_size(THEME.type_scale.caption).text_color(THEME.colors.subtle)
            .child("No catalog yet. Open a session with this harness, then reload choices, or use custom IDs."));
    } else if !route.model.is_empty() && selected_model.is_none() {
        row = row.child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child(
                    "This model is not in the saved catalog and will be skipped. Reload choices after refreshing the harness catalog, or choose a listed model.",
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
        _ => "Effort",
    };
    div()
        .flex_1()
        .when(id == "worker-model", |field| field.flex_grow(2.0))
        .min_w_0()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.muted)
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
                    menu.min_w(gpui::px(180.0))
                        .max_h(gpui::px(320.0))
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
        .gap(THEME.space.sm)
        .p(THEME.space.sm)
        .bg(THEME.colors.surface)
        .rounded(THEME.radius);
    match edit {
        WorkerProfileEdit::Name {
            profile,
            input,
            description,
        } => {
            form = form
                .child(div().child(if profile.is_some() {
                    "Edit profile"
                } else {
                    "New profile"
                }))
                .child(Input::new(input))
                .child(div().child("When to use"))
                .child(Input::new(description))
                .child(
                    div()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.muted)
                        .child("Use letters, numbers, '-' or '_'."),
                );
        }
        WorkerProfileEdit::Custom { inputs, .. } => {
            form = form.child(div().child("Custom IDs"))
                .child(div().text_size(THEME.type_scale.caption).text_color(THEME.colors.muted).child("Use exact IDs for models not listed by the harness. Leave effort blank for its default."))
                .child(div().flex().gap(THEME.space.sm).children(["Provider ID", "Model ID", "Effort"].into_iter().zip(inputs).map(|(label, input)| {
                    div().flex_1().min_w_0().flex().flex_col().gap(THEME.space.xs)
                        .child(div().text_size(THEME.type_scale.caption).text_color(THEME.colors.muted).child(label))
                        .child(Input::new(input))
                })));
        }
    };
    form.child(div().flex().justify_end().gap(THEME.space.sm).child(button(
        "finish-worker-edit",
        "Done",
        ButtonTone::Neutral,
        true,
        move |window, cx| {
            let _ = entity.update(cx, |this, cx| this.finish_worker_profile_edit(window, cx));
        },
    )))
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
