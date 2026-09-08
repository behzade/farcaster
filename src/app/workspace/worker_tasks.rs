use super::*;
use crate::agents::{
    ConfigurationCatalog, WorkerExecution, WorkerJudgment, WorkerTaskDefinition, WorkerTasks,
};

#[derive(Default)]
pub(in crate::app) struct WorkerTaskEditor {
    pub(in crate::app) tasks: Vec<WorkerTaskDefinition>,
    pub(in crate::app) selected: usize,
    pub(in crate::app) edit: Option<WorkerTaskEdit>,
    pub(in crate::app) error: Option<String>,
    loaded: bool,
    saved: Vec<WorkerTaskDefinition>,
    subscriptions: Vec<Subscription>,
    catalogs: Vec<crate::app::persistence::CachedConfigurationCatalog>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) struct WorkerRouteTarget {
    pub(in crate::app) task: usize,
    pub(in crate::app) judgment: WorkerJudgment,
}

pub(in crate::app) enum WorkerTaskEdit {
    Name {
        task: Option<usize>,
        input: Entity<InputState>,
    },
    Custom {
        target: WorkerRouteTarget,
        inputs: [Entity<InputState>; 3],
    },
}

#[derive(Clone)]
pub(in crate::app) enum WorkerRouteChoice {
    Harness(String),
    Provider(String),
    Model { provider: String, id: String },
    Effort(String),
}

impl WorkerTaskEditor {
    fn persist(&mut self, tasks: Vec<WorkerTaskDefinition>) -> Result<(), String> {
        if !self.loaded {
            return Err(
                "worker task settings could not be loaded; reopen Settings before saving".into(),
            );
        }
        let tasks = WorkerTasks { tasks };
        tasks.validate()?;
        if tasks.tasks == self.saved {
            return Ok(());
        }
        let backends = crate::agents::backend_statuses();
        for task in &tasks.tasks {
            for judgment in WorkerJudgment::ALL {
                if !backends
                    .iter()
                    .any(|backend| backend.id == task.execution(judgment).harness)
                {
                    return Err(format!(
                        "unknown worker harness: {}",
                        task.execution(judgment).harness
                    ));
                }
            }
        }
        crate::app::persistence::StateStore::open()?.save_worker_tasks(&tasks)?;
        self.saved = tasks.tasks;
        Ok(())
    }

    fn persist_route(&mut self, target: WorkerRouteTarget) -> Result<(), String> {
        let saved = self.route_settings(target)?;
        self.persist(saved)
    }

    fn route_settings(
        &self,
        target: WorkerRouteTarget,
    ) -> Result<Vec<WorkerTaskDefinition>, String> {
        let route = self
            .tasks
            .get(target.task)
            .ok_or("Task no longer exists")?
            .execution(target.judgment)
            .clone();
        route
            .validate()
            .map_err(|_| "Choose a provider and model to save this route.".to_owned())?;
        let mut saved = self.saved.clone();
        let task = saved.get_mut(target.task).ok_or("Task no longer exists")?;
        match target.judgment {
            WorkerJudgment::Specified => task.specified = route,
            WorkerJudgment::Guided => task.guided = route,
            WorkerJudgment::Independent => task.independent = route,
        }
        Ok(saved)
    }

    pub(in crate::app) fn catalog(&self, harness: &str, project: &Path) -> ConfigurationCatalog {
        let mut result = ConfigurationCatalog::default();
        for entry in &self.catalogs {
            if entry.harness == harness && entry.project == project {
                result.models.extend(entry.catalog.models.clone());
                result.efforts.extend(entry.catalog.efforts.clone());
            }
        }
        result
            .models
            .sort_by(|a, b| (&a.provider, &a.id).cmp(&(&b.provider, &b.id)));
        result
            .models
            .dedup_by(|a, b| a.provider == b.provider && a.id == b.id);
        let mut seen = std::collections::BTreeSet::new();
        result.efforts.retain(|effort| seen.insert(effort.clone()));
        result
    }

    fn route_mut(&mut self, target: WorkerRouteTarget) -> Option<&mut WorkerExecution> {
        let task = self.tasks.get_mut(target.task)?;
        Some(match target.judgment {
            WorkerJudgment::Specified => &mut task.specified,
            WorkerJudgment::Guided => &mut task.guided,
            WorkerJudgment::Independent => &mut task.independent,
        })
    }

    fn save_custom_route(
        &mut self,
        target: WorkerRouteTarget,
        [provider, model, effort]: [String; 3],
    ) -> Result<(), String> {
        let route = self.route_mut(target).ok_or("Task no longer exists")?;
        let next = WorkerExecution {
            harness: route.harness.clone(),
            provider,
            model,
            effort: (!effort.is_empty()).then_some(effort),
        };
        next.validate()?;
        *route = next;
        Ok(())
    }

    fn save_name(&mut self, task: Option<usize>, name: &str) -> Result<(), String> {
        let name = name.trim();
        if !crate::agents::valid_worker_name(name) {
            return Err(
                "Use 1–48 letters, numbers, '-' or '_', starting with a letter or number.".into(),
            );
        }
        if self
            .tasks
            .iter()
            .enumerate()
            .any(|(index, other)| Some(index) != task && other.name.eq_ignore_ascii_case(name))
        {
            return Err(format!("A task named '{name}' already exists."));
        }
        if let Some(index) = task {
            self.tasks
                .get_mut(index)
                .ok_or("Task no longer exists")?
                .name = name.into();
        } else {
            self.tasks.push(WorkerTaskDefinition::new(name.into()));
            self.selected = self.tasks.len() - 1;
        }
        Ok(())
    }
}

fn apply_choice(route: &mut WorkerExecution, choice: WorkerRouteChoice) {
    match choice {
        WorkerRouteChoice::Harness(harness) if route.harness != harness => {
            route.harness = harness;
            route.provider.clear();
            route.model.clear();
            route.effort = None;
        }
        WorkerRouteChoice::Provider(provider) if route.provider != provider => {
            route.provider = provider;
            route.model.clear();
            route.effort = None;
        }
        WorkerRouteChoice::Model { provider, id }
            if route.provider != provider || route.model != id =>
        {
            route.provider = provider;
            route.model = id;
            route.effort = None;
        }
        WorkerRouteChoice::Effort(effort) => route.effort = (!effort.is_empty()).then_some(effort),
        _ => {}
    }
}

pub(in crate::app) fn model_efforts<'a>(
    catalog: &'a ConfigurationCatalog,
    model: Option<&'a crate::protocol::Model>,
) -> &'a [String] {
    match model.filter(|model| model.reasoning) {
        Some(model) => model.efforts.as_deref().unwrap_or(&catalog.efforts),
        None => &[],
    }
}

impl FarcasterApp {
    pub(in crate::app) fn load_worker_task_settings(&mut self) -> Result<(), String> {
        self.worker_task_editor = WorkerTaskEditor::default();
        let store = crate::app::persistence::StateStore::open()?;
        let tasks = store.load_worker_tasks()?.tasks;
        self.worker_task_editor = WorkerTaskEditor {
            saved: tasks.clone(),
            tasks,
            catalogs: store.load_configuration_catalogs()?,
            loaded: true,
            ..WorkerTaskEditor::default()
        };
        Ok(())
    }

    pub(in crate::app) fn reload_worker_choices(&mut self, cx: &mut Context<Self>) {
        match crate::app::persistence::StateStore::open()
            .and_then(|store| store.load_configuration_catalogs())
        {
            Ok(catalogs) => {
                self.worker_task_editor.catalogs = catalogs;
                self.worker_task_editor.error = None;
            }
            Err(error) => self.worker_task_editor.error = Some(error),
        }
        cx.notify();
    }

    pub(in crate::app) fn select_worker_route(
        &mut self,
        target: WorkerRouteTarget,
        choice: WorkerRouteChoice,
        cx: &mut Context<Self>,
    ) {
        if self.worker_task_editor.edit.is_none()
            && let Some(route) = self.worker_task_editor.route_mut(target)
        {
            let previous = route.clone();
            apply_choice(route, choice);
            let valid = route.validate().is_ok();
            let result = self.worker_task_editor.persist_route(target);
            if valid && result.is_err() {
                *self.worker_task_editor.route_mut(target).unwrap() = previous;
            }
            self.worker_task_editor.error = result.err();
        }
        cx.notify();
    }

    pub(in crate::app) fn edit_worker_task_name(
        &mut self,
        task: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.worker_task_editor.edit.is_some() {
            return;
        }
        let name = task
            .and_then(|index| self.worker_task_editor.tasks.get(index))
            .map(|task| task.name.clone())
            .unwrap_or_default();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(name)
                .placeholder("Task name")
        });
        input.read(cx).focus_handle(cx).focus(window, cx);
        self.worker_task_editor.edit = Some(WorkerTaskEdit::Name { task, input });
        self.subscribe_worker_task_inputs(window, cx);
        self.worker_task_editor.error = None;
        cx.notify();
    }

    pub(in crate::app) fn edit_worker_custom_route(
        &mut self,
        target: WorkerRouteTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.worker_task_editor.edit.is_some() {
            return;
        }
        let Some(route) = self.worker_task_editor.route_mut(target) else {
            return;
        };
        let values = [
            route.provider.clone(),
            route.model.clone(),
            route.effort.clone().unwrap_or_default(),
        ];
        let inputs =
            values.map(|value| cx.new(|cx| InputState::new(window, cx).default_value(value)));
        inputs[0].read(cx).focus_handle(cx).focus(window, cx);
        self.worker_task_editor.edit = Some(WorkerTaskEdit::Custom { target, inputs });
        self.subscribe_worker_task_inputs(window, cx);
        self.worker_task_editor.error = None;
        cx.notify();
    }

    pub(in crate::app) fn finish_worker_task_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.save_worker_task_edit(cx) {
            self.worker_task_editor.edit = None;
            self.worker_task_editor.subscriptions.clear();
            self.sheet_focus.focus(window, cx);
            cx.notify();
        }
    }

    fn subscribe_worker_task_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let inputs = match &self.worker_task_editor.edit {
            Some(WorkerTaskEdit::Name { input, .. }) => vec![input.clone()],
            Some(WorkerTaskEdit::Custom { inputs, .. }) => inputs.to_vec(),
            None => return,
        };
        self.worker_task_editor.subscriptions = inputs
            .iter()
            .map(|input| {
                cx.subscribe_in(input, window, |this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.save_worker_task_edit(cx);
                    }
                })
            })
            .collect();
    }

    fn save_worker_task_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let editor = &mut self.worker_task_editor;
        let previous = editor.tasks.clone();
        let selected = editor.selected;
        let result = match &editor.edit {
            Some(WorkerTaskEdit::Name { task, input }) => {
                let (task, name) = (*task, input.read(cx).value().to_string());
                editor.save_name(task, &name).and_then(|()| {
                    let mut saved = editor.saved.clone();
                    if let Some(index) = task {
                        saved[index].name = editor.tasks[index].name.clone();
                    } else {
                        saved.push(editor.tasks.last().unwrap().clone());
                    }
                    editor.persist(saved)?;
                    let index = task.unwrap_or(editor.selected);
                    if let Some(WorkerTaskEdit::Name { task, .. }) = &mut editor.edit {
                        *task = Some(index);
                    }
                    Ok(())
                })
            }
            Some(WorkerTaskEdit::Custom { target, inputs }) => {
                let target = *target;
                let values = inputs
                    .each_ref()
                    .map(|input| input.read(cx).value().trim().to_owned());
                editor
                    .save_custom_route(target, values)
                    .and_then(|()| editor.persist_route(target))
            }
            None => return false,
        };
        let saved = result.is_ok();
        if !saved {
            editor.tasks = previous;
            editor.selected = selected;
        }
        editor.error = result.err();
        cx.notify();
        saved
    }

    pub(in crate::app) fn delete_worker_task(&mut self, cx: &mut Context<Self>) {
        let editor = &mut self.worker_task_editor;
        if editor.edit.is_some() {
            return;
        }
        if editor.selected < editor.tasks.len() {
            let mut saved = editor.saved.clone();
            saved.remove(editor.selected);
            if let Err(error) = editor.persist(saved) {
                editor.error = Some(error);
                cx.notify();
                return;
            }
            editor.tasks.remove(editor.selected);
        }
        editor.selected = editor.selected.min(editor.tasks.len().saturating_sub(1));
        editor.error = None;
        cx.notify();
    }
}

#[cfg(test)]
#[path = "worker_tasks_tests.rs"]
mod tests;
