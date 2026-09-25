use super::*;
use crate::agents::Backend;
use crate::agents::{ConfigurationCatalog, WorkerExecution, WorkerProfile, WorkerProfiles};

#[derive(Default)]
pub(in crate::app) struct WorkerProfileEditor {
    pub(in crate::app) profiles: Vec<WorkerProfile>,
    pub(in crate::app) inherit_selected: bool,
    pub(in crate::app) selected: usize,
    pub(in crate::app) selected_model: usize,
    pub(in crate::app) edit: Option<WorkerProfileEdit>,
    pub(in crate::app) error: Option<String>,
    pub(in crate::app) loaded: bool,
    saved: Vec<WorkerProfile>,
    pub(in crate::app) inherit_limit: usize,
    pub(in crate::app) inherit_enabled: bool,
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
}

pub(in crate::app) enum WorkerProfileEdit {
    Name {
        profile: Option<usize>,
        input: Entity<InputState>,
        description: Entity<InputState>,
        limit: Entity<InputState>,
    },
    Limit {
        profile: Option<usize>,
        input: Entity<InputState>,
    },
    Custom {
        target: WorkerRouteTarget,
        inputs: [Entity<InputState>; 4],
    },
}

#[derive(Clone)]
pub(in crate::app) enum WorkerRouteChoice {
    Harness(Backend),
    Provider(String),
    Model { provider: String, id: String },
    Effort(String),
    ServiceTier(String),
}

impl WorkerProfileEditor {
    pub(in crate::app) fn has_draft(&self) -> bool {
        self.profiles.len() != self.saved.len()
    }

    fn persist(&mut self, profiles: Vec<WorkerProfile>) -> Result<(), String> {
        if !self.loaded {
            return Err(
                "worker profile settings could not be loaded; reopen Settings before saving".into(),
            );
        }
        let profiles = WorkerProfiles {
            profiles,
            inherit_limit: self.inherit_limit,
            inherit_enabled: self.inherit_enabled,
        };
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
        crate::app::persistence::open()?.save_worker_profiles(&profiles)?;
        self.saved = profiles.profiles;
        Ok(())
    }

    fn persist_route(&mut self, target: WorkerRouteTarget) -> Result<(), String> {
        let saved = self.route_settings(target)?;
        self.persist(saved)
    }

    fn route_settings(&self, target: WorkerRouteTarget) -> Result<Vec<WorkerProfile>, String> {
        let draft = self
            .profiles
            .get(target.profile)
            .ok_or("Profile no longer exists")?;
        let route = draft
            .models
            .get(target.model)
            .ok_or("Model no longer exists")?;
        route
            .validate()
            .map_err(|_| "Choose a provider and model to save this route.".to_owned())?;
        let mut saved = self.saved.clone();
        if let Some(profile) = saved.get_mut(target.profile) {
            if profile.models.is_empty() && target.model == 0 {
                profile.models.push(route.clone());
            } else {
                *profile
                    .models
                    .get_mut(target.model)
                    .ok_or("Model no longer exists")? = route.clone();
            }
        } else if target.profile == saved.len() {
            saved.push(draft.clone());
        } else {
            return Err("Profile no longer exists".into());
        }
        Ok(saved)
    }

    pub(in crate::app) fn catalog(&self, harness: Backend, project: &Path) -> ConfigurationCatalog {
        let mut result = ConfigurationCatalog::default();
        for entry in &self.catalogs {
            if entry.profile_id.is_none() && entry.harness == harness && entry.project == project {
                result.models.extend(entry.catalog.models.clone());
                result.efforts.extend(entry.catalog.efforts.clone());
                if result.sandbox_adapter.is_none() {
                    result
                        .sandbox_adapter
                        .clone_from(&entry.catalog.sandbox_adapter);
                }
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
        [provider, model, effort, service_tier]: [String; 4],
    ) -> Result<(), String> {
        let route = self.route_mut(target).ok_or("Profile no longer exists")?;
        let next = WorkerExecution {
            harness: route.harness,
            provider,
            model,
            effort: (!effort.is_empty()).then_some(effort),
            service_tier: (!service_tier.is_empty()).then_some(service_tier),
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
        if name.eq_ignore_ascii_case("inherit") {
            return Err("'inherit' is reserved for Same as caller.".into());
        }
        if self
            .profiles
            .iter()
            .enumerate()
            .any(|(index, other)| Some(index) != profile && other.name.eq_ignore_ascii_case(name))
        {
            return Err(format!("A profile named '{name}' already exists."));
        }
        if profile.is_none() && self.has_draft() {
            return Err("Finish or delete the unsaved profile before adding another.".into());
        }
        if let Some(index) = profile {
            if self.profiles[index].name != name {
                return Err("Profile names stay fixed after creation so existing workers keep their assignment.".into());
            }
            self.profiles
                .get_mut(index)
                .ok_or("Profile no longer exists")?
                .name = name.into();
        } else {
            self.profiles.push(WorkerProfile::new(name.into()));
            self.selected = self.profiles.len() - 1;
            self.inherit_selected = false;
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
    if models.is_empty() && matches!(edit, WorkerModelEdit::Add) {
        models.push(WorkerExecution {
            harness: Backend::Pi,
            provider: String::new(),
            model: String::new(),
            effort: None,
            service_tier: None,
        });
        return Ok(0);
    }
    models.get(index).ok_or("Model no longer exists")?;
    match edit {
        WorkerModelEdit::Add => Err("This profile already has a model.".into()),
        WorkerModelEdit::Remove => {
            models.remove(index);
            Ok(0)
        }
    }
}

fn apply_choice(route: &mut WorkerExecution, choice: WorkerRouteChoice) {
    match choice {
        WorkerRouteChoice::Harness(harness) if route.harness != harness => {
            route.harness = harness;
            route.provider.clear();
            route.model.clear();
            route.effort = None;
            route.service_tier = None;
        }
        WorkerRouteChoice::Provider(provider) if route.provider != provider => {
            route.provider = provider;
            route.model.clear();
            route.effort = None;
            route.service_tier = None;
        }
        WorkerRouteChoice::Model { provider, id }
            if route.provider != provider || route.model != id =>
        {
            route.provider = provider;
            route.model = id;
            route.effort = None;
            route.service_tier = None;
        }
        WorkerRouteChoice::Effort(effort) => route.effort = (!effort.is_empty()).then_some(effort),
        WorkerRouteChoice::ServiceTier(tier) => {
            route.service_tier = (!tier.is_empty()).then_some(tier);
        }
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
        let editor = &mut self.workspace.worker_profile_editor;
        if editor.edit.is_some() {
            return;
        }
        let result = (|| {
            let mut profiles = editor.saved.clone();
            let mut current = editor.profiles.clone();
            let draft = current
                .get_mut(target.profile)
                .ok_or("Profile no longer exists")?;
            // Keep incomplete edits attached to their model as the list moves.
            let selected = if let Some(saved) = profiles.get_mut(target.profile) {
                if saved.models.is_empty() && matches!(edit, WorkerModelEdit::Add) {
                    let selected = edit_models(&mut draft.models, target.model, edit)?;
                    editor.profiles = current;
                    editor.selected_model = selected;
                    return Ok(());
                }
                let selected = edit_models(&mut saved.models, target.model, edit)?;
                edit_models(&mut draft.models, target.model, edit)?;
                editor.persist(profiles)?;
                selected
            } else if target.profile == profiles.len() {
                edit_models(&mut draft.models, target.model, edit)?
            } else {
                return Err("Profile no longer exists".into());
            };
            editor.profiles = current;
            editor.selected_model = selected;
            Ok::<(), String>(())
        })();
        editor.error = result.err();
        cx.notify();
    }

    pub(in crate::app) fn load_worker_profile_settings(&mut self) -> Result<(), String> {
        self.workspace.worker_profile_editor = WorkerProfileEditor::default();
        let store = crate::app::persistence::open()?;
        let mut profiles = store.load_worker_profiles()?;
        profiles
            .profiles
            .sort_by_key(|profile| match profile.name.as_str() {
                "smartest" => 0,
                "smart" => 1,
                "standard" => 2,
                "light" => 3,
                _ => 4,
            });
        let inherit_selected = false;
        self.workspace.worker_profile_editor = WorkerProfileEditor {
            saved: profiles.profiles.clone(),
            profiles: profiles.profiles,
            inherit_selected,
            inherit_limit: profiles.inherit_limit,
            inherit_enabled: profiles.inherit_enabled,
            catalogs: store.load_configuration_catalogs()?,
            loaded: true,
            ..WorkerProfileEditor::default()
        };
        Ok(())
    }

    pub(in crate::app) fn reload_worker_choices(&mut self, cx: &mut Context<Self>) {
        match crate::app::persistence::open().and_then(|store| store.load_configuration_catalogs())
        {
            Ok(catalogs) => {
                self.workspace.worker_profile_editor.catalogs = catalogs;
                self.workspace.worker_profile_editor.error = None;
            }
            Err(error) => self.workspace.worker_profile_editor.error = Some(error),
        }
        cx.notify();
    }

    pub(in crate::app) fn retry_worker_profile_settings(&mut self, cx: &mut Context<Self>) {
        if self.workspace.worker_profile_editor.loaded {
            return;
        }
        if let Err(error) = self.load_worker_profile_settings() {
            self.workspace.worker_profile_editor.error = Some(error);
        }
        cx.notify();
    }

    pub(in crate::app) fn select_worker_route(
        &mut self,
        target: WorkerRouteTarget,
        choice: WorkerRouteChoice,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.worker_profile_editor.edit.is_none()
            && let Some(route) = self.workspace.worker_profile_editor.route_mut(target)
        {
            let previous = route.clone();
            apply_choice(route, choice);
            let valid = route.validate().is_ok();
            let result = self.workspace.worker_profile_editor.persist_route(target);
            if valid && result.is_err() {
                *self
                    .workspace
                    .worker_profile_editor
                    .route_mut(target)
                    .expect("persisting a route preserves its identity") = previous;
            }
            self.workspace.worker_profile_editor.error = result.err();
        }
        cx.notify();
    }

    pub(in crate::app) fn edit_worker_profile(
        &mut self,
        profile: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.worker_profile_editor.edit.is_some() {
            return;
        }
        let current =
            profile.and_then(|index| self.workspace.worker_profile_editor.profiles.get(index));
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
        let limit = current.map_or(10, |profile| profile.limit);
        let limit = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(limit.to_string())
                .placeholder("Maximum active workers")
        });
        if profile.is_some() {
            description.read(cx).focus_handle(cx).focus(window, cx);
        } else {
            input.read(cx).focus_handle(cx).focus(window, cx);
        }
        self.workspace.worker_profile_editor.edit = Some(WorkerProfileEdit::Name {
            profile,
            input,
            description,
            limit,
        });
        self.subscribe_worker_profile_inputs(window, cx);
        self.workspace.worker_profile_editor.error = None;
        cx.notify();
    }

    pub(in crate::app) fn edit_worker_limit(
        &mut self,
        profile: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.worker_profile_editor.edit.is_some() {
            return;
        }
        let editor = &mut self.workspace.worker_profile_editor;
        let limit = profile
            .and_then(|index| editor.profiles.get(index).map(|profile| profile.limit))
            .unwrap_or(editor.inherit_limit);
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(limit.to_string())
                .placeholder("Maximum active workers")
        });
        input.read(cx).focus_handle(cx).focus(window, cx);
        editor.edit = Some(WorkerProfileEdit::Limit { profile, input });
        self.subscribe_worker_profile_inputs(window, cx);
        cx.notify();
    }

    pub(in crate::app) fn toggle_worker_profile(
        &mut self,
        profile: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let editor = &mut self.workspace.worker_profile_editor;
        if editor.edit.is_some() {
            return;
        }
        let mut settings = WorkerProfiles {
            profiles: editor.saved.clone(),
            inherit_limit: editor.inherit_limit,
            inherit_enabled: editor.inherit_enabled,
        };
        if let Some(index) = profile {
            if let Some(item) = settings.profiles.get_mut(index) {
                item.enabled = !item.enabled;
            }
        } else {
            settings.inherit_enabled = !settings.inherit_enabled;
        }
        editor.error = crate::app::persistence::open()
            .and_then(|store| store.save_worker_profiles(&settings))
            .err();
        if editor.error.is_none() {
            editor.inherit_enabled = settings.inherit_enabled;
            editor.saved = settings.profiles.clone();
            if let Some(index) = profile {
                editor.profiles[index].enabled = settings.profiles[index].enabled;
            }
        }
        cx.notify();
    }

    pub(in crate::app) fn edit_worker_custom_route(
        &mut self,
        target: WorkerRouteTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.worker_profile_editor.edit.is_some() {
            return;
        }
        let Some(route) = self.workspace.worker_profile_editor.route_mut(target) else {
            return;
        };
        let values = [
            route.provider.clone(),
            route.model.clone(),
            route.effort.clone().unwrap_or_default(),
            route.service_tier.clone().unwrap_or_default(),
        ];
        let inputs =
            values.map(|value| cx.new(|cx| InputState::new(window, cx).default_value(value)));
        inputs[0].read(cx).focus_handle(cx).focus(window, cx);
        self.workspace.worker_profile_editor.edit =
            Some(WorkerProfileEdit::Custom { target, inputs });
        self.subscribe_worker_profile_inputs(window, cx);
        self.workspace.worker_profile_editor.error = None;
        cx.notify();
    }

    pub(in crate::app) fn finish_worker_profile_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.save_worker_profile_edit(cx) {
            self.workspace.worker_profile_editor.edit = None;
            self.workspace.worker_profile_editor.subscriptions.clear();
            self.overlays.sheet_focus.focus(window, cx);
            cx.notify();
        }
    }

    fn subscribe_worker_profile_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let inputs = match &self.workspace.worker_profile_editor.edit {
            Some(WorkerProfileEdit::Name {
                input,
                description,
                limit,
                ..
            }) => vec![input.clone(), description.clone(), limit.clone()],
            Some(WorkerProfileEdit::Limit { input, .. }) => vec![input.clone()],
            Some(WorkerProfileEdit::Custom { inputs, .. }) => inputs.to_vec(),
            None => return,
        };
        self.workspace.worker_profile_editor.subscriptions = inputs
            .iter()
            .map(|input| {
                cx.subscribe_in(input, window, |this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change)
                        && matches!(
                            this.workspace.worker_profile_editor.edit.as_ref(),
                            Some(WorkerProfileEdit::Custom { .. })
                        )
                    {
                        this.save_worker_profile_edit(cx);
                    }
                })
            })
            .collect();
    }

    fn save_worker_profile_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let editor = &mut self.workspace.worker_profile_editor;
        let previous = editor.profiles.clone();
        let selected = editor.selected;
        let selected_model = editor.selected_model;
        let result = match &editor.edit {
            Some(WorkerProfileEdit::Name {
                profile,
                input,
                description,
                limit,
            }) => {
                let (profile, name) = (*profile, input.read(cx).value().to_string());
                let description = description.read(cx).value().trim().to_owned();
                let limit = limit
                    .read(cx)
                    .value()
                    .parse::<usize>()
                    .ok()
                    .filter(|limit| *limit > 0);
                if description.is_empty() || description.chars().any(char::is_control) {
                    Err("Provide a short description without control characters.".into())
                } else if limit.is_none() {
                    Err("Enter a positive worker limit.".into())
                } else {
                    editor.save_name(profile, &name).and_then(|()| {
                        let index = profile.unwrap_or(editor.selected);
                        editor.profiles[index].description = description;
                        editor.profiles[index].limit = limit.expect("checked above");
                        let mut saved = editor.saved.clone();
                        if index < saved.len() {
                            saved[index].name = editor.profiles[index].name.clone();
                            saved[index].description = editor.profiles[index].description.clone();
                            saved[index].limit = editor.profiles[index].limit;
                            editor.persist(saved)?;
                        } else if index == saved.len() {
                            saved.push(editor.profiles[index].clone());
                            editor.persist(saved)?;
                        } else if index != saved.len() {
                            return Err("Profile no longer exists".into());
                        }
                        if let Some(WorkerProfileEdit::Name { profile, .. }) = &mut editor.edit {
                            *profile = Some(index);
                        }
                        Ok(())
                    })
                }
            }
            Some(WorkerProfileEdit::Limit { profile, input }) => {
                let profile = *profile;
                let limit = input
                    .read(cx)
                    .value()
                    .parse::<usize>()
                    .ok()
                    .filter(|limit| *limit > 0)
                    .ok_or_else(|| "Enter a positive worker limit.".to_owned());
                limit.and_then(|limit| {
                    if let Some(index) = profile {
                        let mut saved = editor.saved.clone();
                        saved
                            .get_mut(index)
                            .ok_or_else(|| "Profile no longer exists".to_owned())?
                            .limit = limit;
                        editor.persist(saved)?;
                        editor.profiles[index].limit = limit;
                    } else {
                        let settings = WorkerProfiles {
                            profiles: editor.saved.clone(),
                            inherit_limit: limit,
                            inherit_enabled: editor.inherit_enabled,
                        };
                        crate::app::persistence::open()?.save_worker_profiles(&settings)?;
                        editor.inherit_limit = limit;
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
        let editor = &mut self.workspace.worker_profile_editor;
        if editor.edit.is_some() {
            return;
        }
        if editor.selected < editor.profiles.len() {
            if matches!(
                editor.profiles[editor.selected].name.as_str(),
                "smartest" | "smart" | "standard" | "light"
            ) {
                editor.error =
                    Some("Built-in profiles cannot be deleted; disable one instead.".into());
                cx.notify();
                return;
            }
            let mut saved = editor.saved.clone();
            if editor.selected < saved.len() {
                saved.remove(editor.selected);
                if let Err(error) = editor.persist(saved) {
                    editor.error = Some(error);
                    cx.notify();
                    return;
                }
            }
            editor.profiles.remove(editor.selected);
        }
        editor.selected = editor.selected.min(editor.profiles.len().saturating_sub(1));
        editor.inherit_selected = editor.profiles.is_empty();
        editor.selected_model = 0;
        editor.error = None;
        cx.notify();
    }
}

#[cfg(test)]
#[path = "worker_tasks_tests.rs"]
mod tests;
