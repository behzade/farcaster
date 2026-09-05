use super::*;
use crate::agents::{WorkerExecution, WorkerJudgment, WorkerTaskDefinition, WorkerTasks};
use gpui::App;

#[derive(Default)]
pub(in crate::app) struct WorkerTaskEditor {
    pub(in crate::app) tasks: Vec<TaskInputs>,
    pub(in crate::app) selected: usize,
    loaded: bool,
    pub(in crate::app) catalogs: Vec<crate::app::persistence::CachedConfigurationCatalog>,
}

pub(in crate::app) struct TaskInputs {
    pub(in crate::app) name: Entity<InputState>,
    pub(in crate::app) routes: [RouteInputs; 3],
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone)]
pub(in crate::app) enum WorkerRouteChoice {
    Harness(String),
    Model(crate::protocol::Model),
    Effort(String),
}

pub(in crate::app) struct RouteInputs {
    pub(in crate::app) harness: String,
    pub(in crate::app) provider: Entity<InputState>,
    pub(in crate::app) model: Entity<InputState>,
    pub(in crate::app) effort: Entity<InputState>,
}

impl WorkerTaskEditor {
    pub(in crate::app) fn new(
        tasks: WorkerTasks,
        window: &mut Window,
        cx: &mut Context<FarcasterApp>,
    ) -> Self {
        Self {
            tasks: tasks
                .tasks
                .into_iter()
                .map(|task| TaskInputs::new(task, window, cx))
                .collect(),
            loaded: true,
            ..Self::default()
        }
    }

    pub(in crate::app) fn value(&self, cx: &App) -> Result<WorkerTasks, String> {
        if !self.loaded {
            return Err(
                "worker task settings could not be loaded; reopen Settings before saving".into(),
            );
        }
        let tasks = WorkerTasks {
            tasks: self
                .tasks
                .iter()
                .map(|task| WorkerTaskDefinition {
                    name: task.name.read(cx).value().trim().to_owned(),
                    specified: task.routes[0].value(cx),
                    guided: task.routes[1].value(cx),
                    independent: task.routes[2].value(cx),
                })
                .collect(),
        };
        tasks.validate()?;
        let backends = crate::agents::backend_statuses();
        for task in &tasks.tasks {
            for judgment in WorkerJudgment::ALL {
                let route = task.execution(judgment);
                if !backends.iter().any(|backend| backend.id == route.harness) {
                    return Err(format!("unknown worker harness: {}", route.harness));
                }
            }
        }
        Ok(tasks)
    }
}

impl TaskInputs {
    fn new(
        task: WorkerTaskDefinition,
        window: &mut Window,
        cx: &mut Context<FarcasterApp>,
    ) -> Self {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(task.name.clone())
                .placeholder("Task name")
        });
        let routes = WorkerJudgment::ALL.map(|judgment| {
            let route = task.execution(judgment);
            RouteInputs {
                harness: route.harness.clone(),
                provider: cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(route.provider.clone())
                        .placeholder("Provider ID")
                }),
                model: cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(route.model.clone())
                        .placeholder("Model ID")
                }),
                effort: cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(route.effort.clone().unwrap_or_default())
                        .placeholder("Backend default")
                }),
            }
        });
        let subscriptions = std::iter::once(&name)
            .chain(
                routes
                    .iter()
                    .flat_map(|route| [&route.provider, &route.model, &route.effort]),
            )
            .map(|input| {
                cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                })
            })
            .collect();
        Self {
            name,
            routes,
            _subscriptions: subscriptions,
        }
    }
}

impl RouteInputs {
    fn value(&self, cx: &App) -> WorkerExecution {
        let effort = self.effort.read(cx).value().trim().to_owned();
        WorkerExecution {
            harness: self.harness.clone(),
            provider: self.provider.read(cx).value().trim().to_owned(),
            model: self.model.read(cx).value().trim().to_owned(),
            effort: (!effort.is_empty()).then_some(effort),
        }
    }
}

impl FarcasterApp {
    pub(in crate::app) fn load_worker_task_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.worker_task_editor = WorkerTaskEditor::default();
        let store = crate::app::persistence::StateStore::open()?;
        let tasks = store.load_worker_tasks()?;
        let mut editor = WorkerTaskEditor::new(tasks, window, cx);
        editor.catalogs = store.load_configuration_catalogs()?;
        self.worker_task_editor = editor;
        Ok(())
    }

    pub(in crate::app) fn select_worker_route(
        &mut self,
        (task, route): (usize, usize),
        choice: WorkerRouteChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(route) = self
            .worker_task_editor
            .tasks
            .get_mut(task)
            .and_then(|task| task.routes.get_mut(route))
        else {
            return;
        };
        match choice {
            WorkerRouteChoice::Harness(harness) if route.harness != harness => {
                route.harness = harness;
                for input in [&route.provider, &route.model, &route.effort] {
                    input.update(cx, |input, cx| input.set_value("", window, cx));
                }
            }
            WorkerRouteChoice::Model(model) => {
                for (input, value) in [
                    (&route.provider, model.provider),
                    (&route.model, model.id),
                    (&route.effort, String::new()),
                ] {
                    input.update(cx, |input, cx| input.set_value(value, window, cx));
                }
            }
            WorkerRouteChoice::Effort(effort) => {
                route
                    .effort
                    .update(cx, |input, cx| input.set_value(effort, window, cx));
            }
            WorkerRouteChoice::Harness(_) => {}
        }
        cx.notify();
    }

    pub(in crate::app) fn add_worker_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut suffix = 1;
        while self.worker_task_editor.tasks.iter().any(|task| {
            task.name
                .read(cx)
                .value()
                .eq_ignore_ascii_case(&format!("task-{suffix}"))
        }) {
            suffix += 1;
        }
        let task = WorkerTaskDefinition::new(format!("task-{suffix}"));
        self.worker_task_editor
            .tasks
            .push(TaskInputs::new(task, window, cx));
        self.worker_task_editor.selected = self.worker_task_editor.tasks.len() - 1;
        cx.notify();
    }

    pub(in crate::app) fn delete_worker_task(&mut self, cx: &mut Context<Self>) {
        let editor = &mut self.worker_task_editor;
        if editor.selected < editor.tasks.len() {
            editor.tasks.remove(editor.selected);
        }
        editor.selected = editor.selected.min(editor.tasks.len().saturating_sub(1));
        cx.notify();
    }
}
