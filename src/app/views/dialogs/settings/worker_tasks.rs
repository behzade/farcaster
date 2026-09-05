use super::*;
use crate::{
    agents::WorkerJudgment,
    app::{
        ui::primitives::dropdown_content_button,
        workspace::worker_tasks::{
            WorkerRouteChoice, WorkerRouteTarget, WorkerTaskEdit, model_efforts,
        },
    },
};
use gpui_component::Disableable as _;

pub(super) fn render(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let editor = &app.worker_task_editor;
    let editing = editor.edit.is_some();
    let reload = entity.clone();
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.md)
        .border_t_1()
        .border_color(THEME.colors.border)
        .pt(THEME.space.md)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(THEME.space.md)
                .child(setting_label(
                    "Worker tasks",
                    "Choose what runs each delegated task. Changes apply to new workers.",
                ))
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
                .child(task_rail(app, entity.clone()))
                .child(div().w(gpui::px(1.0)).bg(THEME.colors.border).flex_none())
                .child(task_detail(app, entity)),
        )
        .into_any_element()
}

fn task_rail(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let editor = &app.worker_task_editor;
    let editing = editor.edit.is_some();
    let add = entity.clone();
    let mut rail = div()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .w(gpui::px(132.0))
        .flex_none();
    for (index, task) in editor.tasks.iter().enumerate() {
        let entity = entity.clone();
        rail = rail.child(
            button(
                ("worker-task", index),
                task_label(&task.name),
                ButtonTone::Quiet,
                !editing,
                move |_, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.worker_task_editor.selected = index;
                        this.worker_task_editor.error = None;
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
            "worker-task-add",
            "+ Add task",
            ButtonTone::Quiet,
            !editing,
            move |window, cx| {
                let _ = add.update(cx, |this, cx| this.edit_worker_task_name(None, window, cx));
            },
        )
        .w_full()
        .justify_start(),
    );

    rail.into_any_element()
}

fn task_detail(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let editor = &app.worker_task_editor;
    let editing = editor.edit.is_some();
    let mut detail = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(THEME.space.sm);
    if let Some(edit @ WorkerTaskEdit::Name { .. }) = &editor.edit {
        detail = detail.child(edit_form(edit, entity.clone()));
    } else if let Some(task) = editor.tasks.get(editor.selected) {
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
                        .child(task_label(&task.name)),
                )
                .child(
                    actions_button("worker-task-actions", "Task actions", !editing)
                        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
                            let rename = rename.clone();
                            let delete = delete.clone();
                            menu.item(PopupMenuItem::new("Rename task…").on_click(
                                move |_, window, cx| {
                                    let _ = rename.update(cx, |this, cx| {
                                        this.edit_worker_task_name(Some(selected), window, cx)
                                    });
                                },
                            ))
                            .item(
                                PopupMenuItem::new("Delete task").on_click(move |_, _, cx| {
                                    let _ =
                                        delete.update(cx, |this, cx| this.delete_worker_task(cx));
                                }),
                            )
                        }),
                ),
        );
        detail = detail.child(
            div().flex().gap(THEME.space.sm).children(
                ["Harness", "Provider", "Model", "Effort"]
                    .into_iter()
                    .map(|label| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.muted)
                            .child(label)
                    }),
            ),
        );
        for judgment in WorkerJudgment::ALL {
            let target = WorkerRouteTarget {
                task: selected,
                judgment,
            };
            detail = detail.child(route(app, entity.clone(), target));
            if let Some(edit @ WorkerTaskEdit::Custom { target: edited, .. }) = &editor.edit
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
                    "Add a task to configure its workers. With no tasks, new workers cannot start.",
                ),
        );
    }
    if let Some(error) = &editor.error {
        detail = detail.child(feedback(
            "worker-task-error",
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
    let editor = &app.worker_task_editor;
    let route = editor.tasks[target.task].execution(target.judgment);
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
    let (label, explanation) = match target.judgment {
        WorkerJudgment::Specified => ("Specified", "Follow the supplied procedure"),
        WorkerJudgment::Guided => ("Guided", "Make local decisions within constraints"),
        WorkerJudgment::Independent => (
            "Independent",
            "Choose an approach and challenge assumptions",
        ),
    };
    let mut row = div()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .pt(THEME.space.sm)
        .border_t_1()
        .border_color(THEME.colors.border)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(THEME.space.sm)
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(THEME.space.sm)
                        .child(div().text_color(THEME.colors.text).child(label))
                        .child(
                            div()
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.subtle)
                                .child(explanation),
                        ),
                )
                .child(
                    actions_button(
                        ("worker-route-actions", target.judgment as usize),
                        format!("{label} route actions"),
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
                    "Current model is not in this catalog. Choose a model or keep the custom ID.",
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
    div()
        .flex_1()
        .min_w_0()
        .child(
            dropdown_content_button(
                (id, target.judgment as usize),
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

fn edit_form(edit: &WorkerTaskEdit, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let mut form = div()
        .flex()
        .flex_col()
        .gap(THEME.space.sm)
        .p(THEME.space.sm)
        .bg(THEME.colors.surface)
        .rounded(THEME.radius);
    let action = match edit {
        WorkerTaskEdit::Name { task, input } => {
            form = form
                .child(div().child(if task.is_some() {
                    "Rename task"
                } else {
                    "New task"
                }))
                .child(Input::new(input))
                .child(
                    div()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.muted)
                        .child("Use letters, numbers, '-' or '_'."),
                );
            if task.is_some() { "Rename" } else { "Add task" }
        }
        WorkerTaskEdit::Custom { inputs, .. } => {
            form = form.child(div().child("Custom IDs"))
                .child(div().text_size(THEME.type_scale.caption).text_color(THEME.colors.muted).child("Use exact IDs for models not listed by the harness. Leave effort blank for its default."))
                .child(div().flex().gap(THEME.space.sm).children(["Provider ID", "Model ID", "Effort"].into_iter().zip(inputs).map(|(label, input)| {
                    div().flex_1().min_w_0().flex().flex_col().gap(THEME.space.xs)
                        .child(div().text_size(THEME.type_scale.caption).text_color(THEME.colors.muted).child(label))
                        .child(Input::new(input))
                })));
            "Apply"
        }
    };
    let apply = entity.clone();
    form.child(
        div()
            .flex()
            .justify_end()
            .gap(THEME.space.sm)
            .child(button(
                "cancel-worker-edit",
                "Cancel",
                ButtonTone::Quiet,
                true,
                move |window, cx| {
                    let _ = entity.update(cx, |this, cx| this.cancel_worker_task_edit(window, cx));
                },
            ))
            .child(button(
                "apply-worker-edit",
                action,
                ButtonTone::Neutral,
                true,
                move |window, cx| {
                    let _ = apply.update(cx, |this, cx| this.apply_worker_task_edit(window, cx));
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

fn task_label(name: &str) -> String {
    match name {
        "read" => "Read".into(),
        "implement" => "Implement".into(),
        "review" => "Review".into(),
        _ => name.into(),
    }
}
