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
    pub(in crate::app) fn value(&self) -> Result<WorkerTasks, String> {
        if !self.loaded {
            return Err(
                "worker task settings could not be loaded; reopen Settings before saving".into(),
            );
        }
        if self.edit.is_some() {
            return Err("Apply or cancel the task edit before saving Settings".into());
        }
        let tasks = WorkerTasks {
            tasks: self.tasks.clone(),
        };
        tasks.validate()?;
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
        Ok(tasks)
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

/// Downstream choices must never survive a change to their provider or harness.
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
    // Known-empty differs from unknown; non-reasoning models have no effort control.
    match model.filter(|model| model.reasoning) {
        Some(model) => model.efforts.as_deref().unwrap_or(&catalog.efforts),
        None => &[],
    }
}

impl FarcasterApp {
    pub(in crate::app) fn load_worker_task_settings(&mut self) -> Result<(), String> {
        self.worker_task_editor = WorkerTaskEditor::default();
        let store = crate::app::persistence::StateStore::open()?;
        self.worker_task_editor = WorkerTaskEditor {
            tasks: store.load_worker_tasks()?.tasks,
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
            apply_choice(route, choice);
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
        self.worker_task_editor.error = None;
        cx.notify();
    }

    pub(in crate::app) fn apply_worker_task_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = match &self.worker_task_editor.edit {
            Some(WorkerTaskEdit::Name { task, input }) => {
                let (task, name) = (*task, input.read(cx).value().to_string());
                self.worker_task_editor.save_name(task, &name)
            }
            Some(WorkerTaskEdit::Custom { target, inputs }) => {
                let target = *target;
                let values = inputs
                    .each_ref()
                    .map(|input| input.read(cx).value().trim().to_owned());
                self.worker_task_editor.save_custom_route(target, values)
            }
            None => return,
        };
        match result {
            Ok(()) => self.cancel_worker_task_edit(window, cx),
            Err(error) => {
                self.worker_task_editor.error = Some(error);
                cx.notify();
            }
        }
    }

    pub(in crate::app) fn cancel_worker_task_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.worker_task_editor.edit = None;
        self.worker_task_editor.error = None;
        self.sheet_focus.focus(window, cx);
        cx.notify();
    }

    pub(in crate::app) fn delete_worker_task(&mut self, cx: &mut Context<Self>) {
        let editor = &mut self.worker_task_editor;
        if editor.edit.is_some() {
            return;
        }
        if editor.selected < editor.tasks.len() {
            editor.tasks.remove(editor.selected);
        }
        editor.selected = editor.selected.min(editor.tasks.len().saturating_sub(1));
        editor.error = None;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_route_changes_clear_only_downstream_choices() {
        let mut route = WorkerTaskDefinition::new("read".into()).specified;
        let original = route.clone();
        apply_choice(
            &mut route,
            WorkerRouteChoice::Provider(original.provider.clone()),
        );
        assert_eq!(route, original);
        apply_choice(
            &mut route,
            WorkerRouteChoice::Model {
                provider: original.provider.clone(),
                id: original.model.clone(),
            },
        );
        assert_eq!(route, original);
        apply_choice(
            &mut route,
            WorkerRouteChoice::Model {
                provider: original.provider.clone(),
                id: "another-model".into(),
            },
        );
        assert_eq!(route.provider, original.provider);
        assert_eq!(route.effort, None);
        apply_choice(&mut route, WorkerRouteChoice::Provider("other".into()));
        assert_eq!(route.harness, original.harness);
        assert!(route.model.is_empty());
        assert_eq!(route.effort, None);
        apply_choice(&mut route, WorkerRouteChoice::Harness("codex-cli".into()));
        assert!(route.provider.is_empty());
    }

    #[test]
    fn worker_task_edits_validate_before_mutating() {
        let mut editor = WorkerTaskEditor::default();
        assert!(editor.save_name(None, "bad name").is_err());
        assert!(editor.tasks.is_empty());
        editor.save_name(None, "audit").unwrap();
        assert!(editor.save_name(None, "AUDIT").is_err());
        editor.save_name(Some(0), "review").unwrap();
        assert_eq!(editor.tasks.len(), 1);
        assert_eq!(editor.tasks[0].name, "review");
        let target = WorkerRouteTarget {
            task: 0,
            judgment: WorkerJudgment::Guided,
        };
        let original = editor.tasks[0].guided.clone();
        assert!(
            editor
                .save_custom_route(target, ["provider".into(), String::new(), "high".into()])
                .is_err()
        );
        assert_eq!(editor.tasks[0].guided, original);
        editor
            .save_custom_route(
                target,
                ["provider".into(), "custom-model".into(), String::new()],
            )
            .unwrap();
        assert_eq!(editor.tasks[0].guided.harness, original.harness);
        assert_eq!(editor.tasks[0].guided.model, "custom-model");
        assert_eq!(editor.tasks[0].guided.effort, None);
    }

    #[test]
    fn worker_catalogs_preserve_effort_order_and_project_scope() {
        let entry = |harness: &str, project: &str, efforts: &[&str]| {
            crate::app::persistence::CachedConfigurationCatalog {
                harness: harness.into(),
                project: project.into(),
                catalog: ConfigurationCatalog {
                    models: vec![],
                    efforts: efforts.iter().map(|value| (*value).into()).collect(),
                },
            }
        };
        let editor = WorkerTaskEditor {
            catalogs: vec![
                entry("pi", "/project", &["low", "medium", "high"]),
                entry("pi", "/other", &["wrong"]),
                entry("codex-cli", "/project", &["wrong"]),
                entry("pi", "/project", &["high"]),
            ],
            ..WorkerTaskEditor::default()
        };
        assert_eq!(
            editor.catalog("pi", Path::new("/project")).efforts,
            ["low", "medium", "high"]
        );
    }

    #[test]
    fn worker_efforts_follow_the_selected_model_not_the_harness_alone() {
        let route = WorkerTaskDefinition::new("read".into()).specified;
        let mut catalog = ConfigurationCatalog {
            models: vec![crate::protocol::Model {
                id: route.model.clone(),
                name: "Luna".into(),
                provider: route.provider.clone(),
                context_window: 0,
                reasoning: true,
                efforts: Some(vec!["high".into()]),
            }],
            efforts: vec!["low".into(), "high".into()],
        };
        assert_eq!(model_efforts(&catalog, catalog.models.first()), ["high"]);
        catalog.models[0].efforts = Some(vec![]);
        assert!(model_efforts(&catalog, catalog.models.first()).is_empty());
        catalog.models[0].efforts = None;
        assert_eq!(
            model_efforts(&catalog, catalog.models.first()),
            ["low", "high"]
        );
        catalog.models[0].reasoning = false;
        assert!(model_efforts(&catalog, catalog.models.first()).is_empty());
    }
}
