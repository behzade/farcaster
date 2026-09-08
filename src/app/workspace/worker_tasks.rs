use super::*;
use crate::agents::{ConfigurationCatalog, WorkerExecution, WorkerProfile, WorkerProfiles};

#[derive(Default)]
pub(in crate::app) struct WorkerProfileEditor {
    pub(in crate::app) profiles: Vec<WorkerProfile>,
    pub(in crate::app) selected: usize,
    pub(in crate::app) selected_model: usize,
    pub(in crate::app) edit: Option<WorkerProfileEdit>,
    pub(in crate::app) error: Option<String>,
    loaded: bool,
    saved: Vec<WorkerProfile>,
    subscriptions: Vec<Subscription>,
    catalogs: Vec<crate::app::persistence::CachedConfigurationCatalog>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) struct WorkerRouteTarget {
    pub(in crate::app) profile: usize,
    pub(in crate::app) model: usize,
}

#[derive(Clone, Copy)]
pub(in crate::app) enum WorkerModelEdit {
    Add,
    Remove,
    MoveUp,
    MoveDown,
}

pub(in crate::app) enum WorkerProfileEdit {
    Name {
        profile: Option<usize>,
        input: Entity<InputState>,
        description: Entity<InputState>,
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

impl WorkerProfileEditor {
    fn persist(&mut self, profiles: Vec<WorkerProfile>) -> Result<(), String> {
        if !self.loaded {
            return Err(
                "worker profile settings could not be loaded; reopen Settings before saving".into(),
            );
        }
        let profiles = WorkerProfiles { profiles };
        profiles.validate()?;
        if profiles.profiles == self.saved {
            return Ok(());
        }
        let backends = crate::agents::backend_statuses();
        for profile in &profiles.profiles {
            for model in &profile.models {
                if !backends.iter().any(|backend| backend.id == model.harness) {
                    return Err(format!("unknown worker harness: {}", model.harness));
                }
            }
        }
        crate::app::persistence::StateStore::open()?.save_worker_profiles(&profiles)?;
        self.saved = profiles.profiles;
        Ok(())
    }

    fn persist_route(&mut self, target: WorkerRouteTarget) -> Result<(), String> {
        let saved = self.route_settings(target)?;
        self.persist(saved)
    }

    fn route_settings(&self, target: WorkerRouteTarget) -> Result<Vec<WorkerProfile>, String> {
        let route = self
            .profiles
            .get(target.profile)
            .ok_or("Profile no longer exists")?
            .models
            .get(target.model)
            .ok_or("Model no longer exists")?
            .clone();
        route
            .validate()
            .map_err(|_| "Choose a provider and model to save this route.".to_owned())?;
        let mut saved = self.saved.clone();
        let profile = saved
            .get_mut(target.profile)
            .ok_or("Profile no longer exists")?;
        *profile
            .models
            .get_mut(target.model)
            .ok_or("Model no longer exists")? = route;
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
        let profile = self.profiles.get_mut(target.profile)?;
        profile.models.get_mut(target.model)
    }

    fn save_custom_route(
        &mut self,
        target: WorkerRouteTarget,
        [provider, model, effort]: [String; 3],
    ) -> Result<(), String> {
        let route = self.route_mut(target).ok_or("Profile no longer exists")?;
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

    fn save_name(&mut self, profile: Option<usize>, name: &str) -> Result<(), String> {
        let name = name.trim();
        if !crate::agents::valid_worker_name(name) {
            return Err(
                "Use 1–48 letters, numbers, '-' or '_', starting with a letter or number.".into(),
            );
        }
        if self
            .profiles
            .iter()
            .enumerate()
            .any(|(index, other)| Some(index) != profile && other.name.eq_ignore_ascii_case(name))
        {
            return Err(format!("A profile named '{name}' already exists."));
        }
        if let Some(index) = profile {
            self.profiles
                .get_mut(index)
                .ok_or("Profile no longer exists")?
                .name = name.into();
        } else {
            self.profiles.push(WorkerProfile::new(name.into()));
            self.selected = self.profiles.len() - 1;
            self.selected_model = 0;
        }
        Ok(())
    }
}

fn edit_models(
    models: &mut Vec<WorkerExecution>,
    index: usize,
    edit: WorkerModelEdit,
) -> Result<usize, String> {
    let model = models.get(index).ok_or("Model no longer exists")?;
    match edit {
        WorkerModelEdit::Add => {
            models.push(model.clone());
            Ok(models.len() - 1)
        }
        WorkerModelEdit::Remove if models.len() > 1 => {
            models.remove(index);
            Ok(index.min(models.len() - 1))
        }
        WorkerModelEdit::MoveUp if index > 0 => {
            models.swap(index, index - 1);
            Ok(index - 1)
        }
        WorkerModelEdit::MoveDown if index + 1 < models.len() => {
            models.swap(index, index + 1);
            Ok(index + 1)
        }
        _ => Err("Keep at least one model and move models only within the list.".into()),
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
    pub(in crate::app) fn edit_worker_models(
        &mut self,
        target: WorkerRouteTarget,
        edit: WorkerModelEdit,
        cx: &mut Context<Self>,
    ) {
        let editor = &mut self.worker_profile_editor;
        if editor.edit.is_some() {
            return;
        }
        let result = (|| {
            let mut profiles = editor.saved.clone();
            let mut current = editor.profiles.clone();
            let saved = profiles
                .get_mut(target.profile)
                .ok_or("Profile no longer exists")?;
            let draft = current
                .get_mut(target.profile)
                .ok_or("Profile no longer exists")?;
            // Keep incomplete edits attached to their model as the list moves.
            let selected = edit_models(&mut saved.models, target.model, edit)?;
            edit_models(&mut draft.models, target.model, edit)?;
            if matches!(edit, WorkerModelEdit::Add) {
                draft.models[selected] = saved.models[selected].clone();
            }
            editor.persist(profiles)?;
            editor.profiles = current;
            editor.selected_model = selected;
            Ok::<(), String>(())
        })();
        editor.error = result.err();
        cx.notify();
    }

    pub(in crate::app) fn load_worker_profile_settings(&mut self) -> Result<(), String> {
        self.worker_profile_editor = WorkerProfileEditor::default();
        let store = crate::app::persistence::StateStore::open()?;
        let profiles = store.load_worker_profiles()?.profiles;
        self.worker_profile_editor = WorkerProfileEditor {
            saved: profiles.clone(),
            profiles,
            catalogs: store.load_configuration_catalogs()?,
            loaded: true,
            ..WorkerProfileEditor::default()
        };
        Ok(())
    }

    pub(in crate::app) fn reload_worker_choices(&mut self, cx: &mut Context<Self>) {
        match crate::app::persistence::StateStore::open()
            .and_then(|store| store.load_configuration_catalogs())
        {
            Ok(catalogs) => {
                self.worker_profile_editor.catalogs = catalogs;
                self.worker_profile_editor.error = None;
            }
            Err(error) => self.worker_profile_editor.error = Some(error),
        }
        cx.notify();
    }

    pub(in crate::app) fn select_worker_route(
        &mut self,
        target: WorkerRouteTarget,
        choice: WorkerRouteChoice,
        cx: &mut Context<Self>,
    ) {
        if self.worker_profile_editor.edit.is_none()
            && let Some(route) = self.worker_profile_editor.route_mut(target)
        {
            let previous = route.clone();
            apply_choice(route, choice);
            let valid = route.validate().is_ok();
            let result = self.worker_profile_editor.persist_route(target);
            if valid && result.is_err() {
                *self.worker_profile_editor.route_mut(target).unwrap() = previous;
            }
            self.worker_profile_editor.error = result.err();
        }
        cx.notify();
    }

    pub(in crate::app) fn edit_worker_profile(
        &mut self,
        profile: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.worker_profile_editor.edit.is_some() {
            return;
        }
        let current = profile.and_then(|index| self.worker_profile_editor.profiles.get(index));
        let name = current
            .map(|profile| profile.name.clone())
            .unwrap_or_default();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(name)
                .placeholder("Profile name")
        });
        let description = current
            .map(|profile| profile.description.clone())
            .unwrap_or_default();
        let description = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(description)
                .placeholder("When should the agent choose this worker?")
        });
        input.read(cx).focus_handle(cx).focus(window, cx);
        self.worker_profile_editor.edit = Some(WorkerProfileEdit::Name {
            profile,
            input,
            description,
        });
        self.subscribe_worker_profile_inputs(window, cx);
        self.worker_profile_editor.error = None;
        cx.notify();
    }

    pub(in crate::app) fn edit_worker_custom_route(
        &mut self,
        target: WorkerRouteTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.worker_profile_editor.edit.is_some() {
            return;
        }
        let Some(route) = self.worker_profile_editor.route_mut(target) else {
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
        self.worker_profile_editor.edit = Some(WorkerProfileEdit::Custom { target, inputs });
        self.subscribe_worker_profile_inputs(window, cx);
        self.worker_profile_editor.error = None;
        cx.notify();
    }

    pub(in crate::app) fn finish_worker_profile_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.save_worker_profile_edit(cx) {
            self.worker_profile_editor.edit = None;
            self.worker_profile_editor.subscriptions.clear();
            self.sheet_focus.focus(window, cx);
            cx.notify();
        }
    }

    fn subscribe_worker_profile_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let inputs = match &self.worker_profile_editor.edit {
            Some(WorkerProfileEdit::Name {
                input, description, ..
            }) => vec![input.clone(), description.clone()],
            Some(WorkerProfileEdit::Custom { inputs, .. }) => inputs.to_vec(),
            None => return,
        };
        self.worker_profile_editor.subscriptions = inputs
            .iter()
            .map(|input| {
                cx.subscribe_in(input, window, |this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.save_worker_profile_edit(cx);
                    }
                })
            })
            .collect();
    }

    fn save_worker_profile_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let editor = &mut self.worker_profile_editor;
        let previous = editor.profiles.clone();
        let selected = editor.selected;
        let selected_model = editor.selected_model;
        let result = match &editor.edit {
            Some(WorkerProfileEdit::Name {
                profile,
                input,
                description,
            }) => {
                let (profile, name) = (*profile, input.read(cx).value().to_string());
                let description = description.read(cx).value().trim().to_owned();
                editor.save_name(profile, &name).and_then(|()| {
                    let index = profile.unwrap_or(editor.selected);
                    editor.profiles[index].description = description;
                    let mut saved = editor.saved.clone();
                    if let Some(index) = profile {
                        saved[index].name = editor.profiles[index].name.clone();
                        saved[index].description = editor.profiles[index].description.clone();
                    } else {
                        saved.push(editor.profiles.last().unwrap().clone());
                    }
                    editor.persist(saved)?;
                    if let Some(WorkerProfileEdit::Name { profile, .. }) = &mut editor.edit {
                        *profile = Some(index);
                    }
                    Ok(())
                })
            }
            Some(WorkerProfileEdit::Custom { target, inputs }) => {
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
            editor.profiles = previous;
            editor.selected = selected;
            editor.selected_model = selected_model;
        }
        editor.error = result.err();
        cx.notify();
        saved
    }

    pub(in crate::app) fn delete_worker_profile(&mut self, cx: &mut Context<Self>) {
        let editor = &mut self.worker_profile_editor;
        if editor.edit.is_some() {
            return;
        }
        if editor.selected < editor.profiles.len() {
            let mut saved = editor.saved.clone();
            saved.remove(editor.selected);
            if let Err(error) = editor.persist(saved) {
                editor.error = Some(error);
                cx.notify();
                return;
            }
            editor.profiles.remove(editor.selected);
        }
        editor.selected = editor.selected.min(editor.profiles.len().saturating_sub(1));
        editor.selected_model = 0;
        editor.error = None;
        cx.notify();
    }
}

#[cfg(test)]
#[path = "worker_tasks_tests.rs"]
mod tests;
