use super::*;
use crate::agents::WorkerJudgment;
use crate::app::workspace::worker_tasks::WorkerRouteChoice;

pub(super) fn render(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>, cx: &App) -> AnyElement {
    let editor = &app.worker_task_editor;
    let selected = editor.selected;
    let names = editor
        .tasks
        .iter()
        .map(|task| task.name.read(cx).value().to_string())
        .collect::<Vec<_>>();
    let select = entity.clone();
    let add = entity.clone();
    let delete = entity.clone();
    let mut content = div().flex().flex_col().gap(THEME.space.sm)
        .border_t_1().border_color(THEME.colors.border).pt(THEME.space.md)
        .child(setting_label("Worker tasks", "Routes already-delegated work. Changes affect new children only; task labels do not enforce permissions."))
        .child(div().flex().items_center().gap(THEME.space.sm)
            .child(dropdown_button("worker-task-select", names.get(selected).cloned().unwrap_or_else(|| "No tasks".into()), ButtonTone::Neutral, !names.is_empty())
                .dropdown_menu_with_anchor(gpui::Anchor::TopLeft, move |menu, _, _| {
                    names.iter().enumerate().fold(menu, |menu, (index, name)| {
                        let entity = select.clone();
                        menu.item(PopupMenuItem::new(name.clone()).checked(index == selected).on_click(move |_, _, cx| {
                            let _ = entity.update(cx, |this, cx| { this.worker_task_editor.selected = index; cx.notify(); });
                        }))
                    })
                }))
            .child(button("worker-task-add", "Add task", ButtonTone::Neutral, true, move |window, cx| {
                let _ = add.update(cx, |this, cx| this.add_worker_task(window, cx));
            }))
            .child(button("worker-task-delete", "Delete task", ButtonTone::Danger, !editor.tasks.is_empty(), move |_, cx| {
                let _ = delete.update(cx, |this, cx| this.delete_worker_task(cx));
            })));
    if let Some(task) = editor.tasks.get(selected) {
        content = content.child(field(
            "Task name",
            Input::new(&task.name).into_any_element(),
        ));
        for (index, judgment) in WorkerJudgment::ALL.into_iter().enumerate() {
            content = content.child(route(app, entity.clone(), (selected, index), judgment, cx));
        }
        content = content.child(div().text_size(THEME.type_scale.caption).text_color(THEME.colors.subtle)
            .child("Choose from cached harness catalogs or enter exact IDs. Farcaster never substitutes the parent's model."));
    } else {
        content = content.child(div().text_color(THEME.colors.subtle).child("No task definitions. Existing children can still exchange messages; new children cannot start."));
    }
    content.into_any_element()
}

fn route(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
    target: (usize, usize),
    judgment: WorkerJudgment,
    cx: &App,
) -> AnyElement {
    let editor = &app.worker_task_editor;
    let route = &editor.tasks[target.0].routes[target.1];
    let catalogs = editor
        .catalogs
        .iter()
        .filter(|entry| entry.harness == route.harness && entry.project == app.project)
        .collect::<Vec<_>>();
    let mut models = catalogs
        .iter()
        .flat_map(|entry| entry.catalog.models.clone())
        .collect::<Vec<_>>();
    models.sort_by(|a, b| (&a.provider, &a.id).cmp(&(&b.provider, &b.id)));
    models.dedup_by(|a, b| a.provider == b.provider && a.id == b.id);
    let efforts = models
        .iter()
        .find(|model| {
            model.provider == &*route.provider.read(cx).value()
                && model.id == &*route.model.read(cx).value()
        })
        .and_then(|model| model.efforts.clone())
        .unwrap_or_else(|| {
            catalogs
                .iter()
                .flat_map(|entry| entry.catalog.efforts.clone())
                .collect()
        });
    let harnesses = crate::agents::backend_statuses()
        .into_iter()
        .map(|backend| {
            let label = if backend.available {
                backend.name
            } else {
                format!("{} (not installed)", backend.name)
            };
            (
                label,
                backend.id == route.harness,
                WorkerRouteChoice::Harness(backend.id),
            )
        })
        .collect();
    let models = models
        .into_iter()
        .map(|model| {
            (
                format!("{} / {}", model.provider, model.id),
                false,
                WorkerRouteChoice::Model(model),
            )
        })
        .collect();
    let efforts = std::iter::once(String::new())
        .chain(efforts)
        .map(|effort| {
            (
                if effort.is_empty() {
                    "Backend default".into()
                } else {
                    effort.clone()
                },
                false,
                WorkerRouteChoice::Effort(effort),
            )
        })
        .collect();
    let description = match judgment {
        WorkerJudgment::Specified => "Parent supplies the procedure or exact checks",
        WorkerJudgment::Guided => "Child makes local decisions within constraints",
        WorkerJudgment::Independent => "Child chooses an approach or challenges assumptions",
    };
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.sm)
        .p(THEME.space.sm)
        .border_1()
        .border_color(THEME.colors.border)
        .rounded(THEME.radius)
        .child(setting_label(judgment.label(), description))
        .child(
            div()
                .flex()
                .gap(THEME.space.sm)
                .child(field(
                    "Harness",
                    route_menu(
                        "worker-harness",
                        crate::agents::backend_display_name(&route.harness),
                        harnesses,
                        target,
                        entity.clone(),
                    ),
                ))
                .child(field(
                    "Provider",
                    Input::new(&route.provider).into_any_element(),
                )),
        )
        .child(
            div()
                .flex()
                .items_end()
                .gap(THEME.space.sm)
                .child(field("Model", Input::new(&route.model).into_any_element()))
                .child(route_menu(
                    "worker-model",
                    "Choose model",
                    models,
                    target,
                    entity.clone(),
                )),
        )
        .child(
            div()
                .flex()
                .items_end()
                .gap(THEME.space.sm)
                .child(field(
                    "Effort",
                    Input::new(&route.effort).into_any_element(),
                ))
                .child(route_menu(
                    "worker-effort",
                    "Choose effort",
                    efforts,
                    target,
                    entity,
                )),
        )
        .into_any_element()
}

fn route_menu(
    id: &'static str,
    label: impl Into<gpui::SharedString>,
    choices: Vec<(String, bool, WorkerRouteChoice)>,
    target: (usize, usize),
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    dropdown_button(
        (id, target.1),
        label,
        ButtonTone::Neutral,
        !choices.is_empty(),
    )
    .dropdown_menu_with_anchor(gpui::Anchor::TopLeft, move |menu, _, _| {
        choices.iter().fold(menu, |menu, (label, checked, choice)| {
            let entity = entity.clone();
            let choice = choice.clone();
            menu.item(
                PopupMenuItem::new(label.clone())
                    .checked(*checked)
                    .on_click(move |_, window, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.select_worker_route(target, choice.clone(), window, cx)
                        });
                    }),
            )
        })
    })
    .into_any_element()
}

fn field(label: &'static str, input: AnyElement) -> AnyElement {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.muted)
                .child(label),
        )
        .child(input)
        .into_any_element()
}
