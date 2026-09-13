use std::{collections::HashMap, path::PathBuf};

use crate::agents::effort_rank;
use crate::protocol::Model;

use super::{ConfigurationStatus, RuntimeSnapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SessionIdentity<'a> {
    pub provider: Option<&'a str>,
    pub model: Option<&'a Model>,
    pub effort: Option<&'a str>,
}

pub(super) fn replacement_effort(model: &Model, current: Option<&str>) -> Option<String> {
    let current = current?;
    let efforts = model.efforts.as_deref()?;
    if efforts.iter().any(|effort| effort == current) {
        return None;
    }
    let current_rank = effort_rank(current);
    efforts
        .iter()
        .min_by_key(|effort| match (current_rank, effort_rank(effort)) {
            (Some(current), Some(candidate)) => current.abs_diff(candidate),
            _ => u8::MAX,
        })
        .cloned()
}

impl RuntimeSnapshot {
    pub(crate) fn available_access_modes(&self) -> Vec<crate::agents::HarnessAccessMode> {
        crate::agents::available_access_modes(
            &self.harness,
            self.access_mode_model(),
            self.sandbox_adapter.as_deref(),
        )
    }

    pub(crate) fn sandbox_controls_available(&self) -> bool {
        !crate::agents::supports_sandbox_discovery(&self.harness) || self.sandbox_adapter.is_some()
    }

    pub(crate) fn access_mode_for_new_session(&self) -> crate::agents::HarnessAccessMode {
        if self.sandbox_controls_available() {
            self.access_mode
        } else {
            // Discovery, not a previous unmanaged session, chooses the new mode.
            crate::agents::HarnessAccessMode::default()
        }
    }

    pub(super) fn access_mode_model(&self) -> Option<&Model> {
        self.session_identity()
            .model
            .map(|model| self.catalog_model(model))
            .or_else(|| self.models.first())
    }

    pub(crate) fn catalog_model<'a>(&'a self, selected: &'a Model) -> &'a Model {
        self.models
            .iter()
            .find(|model| model.id == selected.id && model.provider == selected.provider)
            .or_else(|| {
                self.models.iter().find(|model| {
                    model.provider == selected.provider
                        && model.resolved_model.as_deref() == Some(selected.id.as_str())
                })
            })
            .unwrap_or(selected)
    }

    pub(crate) fn session_target(&self) -> Option<crate::sessions::SessionTarget> {
        let state = self.session.as_ref()?;
        if self.harness.is_empty() || state.session_id.is_empty() {
            return None;
        }
        Some(crate::sessions::SessionTarget {
            harness: self.harness.clone(),
            id: state.session_id.clone(),
            path: crate::sessions::normalize_session_path(self.selected_session.as_deref()?),
        })
    }

    pub(crate) fn session_identity(&self) -> SessionIdentity<'_> {
        let model = self
            .session
            .as_ref()
            .and_then(|session| session.model.as_ref())
            .or(self.prefill_model.as_ref());
        // A live session's unset effort is authoritative, not a missing draft value.
        let effort = match &self.session {
            Some(session) => session.thinking_level.as_deref(),
            None => self.prefill_thinking_level.as_deref(),
        }
        .filter(|level| !level.is_empty());
        SessionIdentity {
            provider: model.map(|model| model.provider.as_str()),
            model,
            effort,
        }
    }

    pub(crate) fn effort_choices(&self, model: &Model) -> Vec<Option<String>> {
        crate::agents::supports_reasoning_reset(&self.harness)
            .then_some(None)
            .into_iter()
            .chain(
                crate::agents::model_efforts(self.catalog_model(model), &self.thinking_levels)
                    .into_iter()
                    .map(Some),
            )
            .collect()
    }

    pub(crate) fn available_thinking_levels(&self) -> &[String] {
        let Some(selected) = self
            .session_identity()
            .model
            .or_else(|| self.models.first())
        else {
            return &self.thinking_levels;
        };
        let model = self.catalog_model(selected);
        if !model.reasoning {
            return &[];
        }
        match model.efforts.as_deref() {
            Some(efforts) => efforts,
            None => &self.thinking_levels,
        }
    }
}

#[derive(Default)]
pub(super) struct HarnessConfigurationStore {
    identities: HashMap<String, OwnedSessionIdentity>,
    catalogs: HashMap<(String, PathBuf), HarnessCatalog>,
}

#[derive(Default)]
struct HarnessCatalog {
    models: Vec<Model>,
    efforts: Vec<String>,
    sandbox_adapter: Option<String>,
    status: ConfigurationStatus,
}

#[derive(Default)]
struct OwnedSessionIdentity {
    model: Option<Model>,
    effort: Option<String>,
}

impl HarnessConfigurationStore {
    pub fn restore(
        &mut self,
        entries: Vec<crate::app::infrastructure::persistence::CachedSessionControlDefaults>,
    ) {
        for entry in entries {
            let effort = entry
                .effort
                .filter(|_| crate::agents::supports_reasoning_effort(&entry.harness));
            self.identities.insert(
                entry.harness,
                OwnedSessionIdentity {
                    model: entry.model,
                    effort,
                },
            );
        }
    }

    pub fn cached(
        &self,
    ) -> Vec<crate::app::infrastructure::persistence::CachedSessionControlDefaults> {
        let mut entries = self
            .identities
            .iter()
            .filter(|(_, identity)| identity.model.is_some() || identity.effort.is_some())
            .map(|(harness, identity)| {
                crate::app::infrastructure::persistence::CachedSessionControlDefaults {
                    harness: harness.clone(),
                    model: identity.model.clone(),
                    effort: identity.effort.clone(),
                }
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.harness.cmp(&right.harness));
        entries
    }

    pub fn model(&self, harness: &str) -> Option<&Model> {
        self.identities
            .get(harness)
            .and_then(|identity| identity.model.as_ref())
    }

    pub fn effort(&self, harness: &str) -> Option<&str> {
        self.identities
            .get(harness)
            .and_then(|identity| identity.effort.as_deref())
    }

    pub fn set_model(&mut self, harness: &str, model: Model) -> bool {
        let identity = self.identities.entry(harness.to_owned()).or_default();
        let replacement = replacement_effort(&model, identity.effort.as_deref());
        let changed = identity.model.as_ref() != Some(&model) || replacement.is_some();
        identity.model = Some(model);
        if let Some(effort) = replacement {
            identity.effort = Some(effort);
        }
        changed
    }

    pub fn set_effort(&mut self, harness: &str, effort: String) -> bool {
        if !crate::agents::supports_reasoning_effort(harness) {
            return false;
        }
        let identity = self.identities.entry(harness.to_owned()).or_default();
        if identity.effort.as_ref() == Some(&effort) {
            return false;
        }
        identity.effort = Some(effort);
        true
    }

    pub fn reset_effort(&mut self, harness: &str) -> bool {
        if !crate::agents::supports_reasoning_reset(harness) {
            return false;
        }
        self.identities
            .entry(harness.to_owned())
            .or_default()
            .effort
            .take()
            .is_some()
    }

    pub fn set_catalog(
        &mut self,
        harness: String,
        project: PathBuf,
        catalog: crate::agents::ConfigurationCatalog,
    ) {
        let cached = self.catalogs.entry((harness, project)).or_default();
        cached.models = catalog.models;
        cached.efforts = catalog.efforts;
        cached.sandbox_adapter = catalog.sandbox_adapter;
        cached.status = ConfigurationStatus::Loaded;
    }

    pub fn set_catalog_loading(&mut self, harness: String, project: PathBuf) {
        self.catalogs.entry((harness, project)).or_default().status = ConfigurationStatus::Loading;
    }

    pub fn set_catalog_error(&mut self, harness: String, project: PathBuf, error: String) {
        self.catalogs.entry((harness, project)).or_default().status =
            ConfigurationStatus::Failed(error);
    }

    pub fn catalog_command(
        &self,
        harness: &str,
        project: &std::path::Path,
    ) -> Option<super::RuntimeCommand> {
        let catalog = self
            .catalogs
            .get(&(harness.to_owned(), project.to_owned()))?;
        if catalog.models.is_empty() && catalog.sandbox_adapter.is_none() {
            return None;
        }
        Some(super::RuntimeCommand::UpdateConfigurationCatalog {
            harness: harness.to_owned(),
            project: project.to_owned(),
            catalog: crate::agents::ConfigurationCatalog {
                models: catalog.models.clone(),
                efforts: catalog.efforts.clone(),
                sandbox_adapter: catalog.sandbox_adapter.clone(),
            },
        })
    }

    pub fn catalog_command_for_snapshot(
        &self,
        snapshot: &RuntimeSnapshot,
    ) -> Option<super::RuntimeCommand> {
        if snapshot.connected {
            return None;
        }
        let command = self.catalog_command(&snapshot.harness, &snapshot.project)?;
        let super::RuntimeCommand::UpdateConfigurationCatalog { catalog, .. } = &command else {
            unreachable!("catalog_command returned a non-catalog command")
        };
        let current = snapshot.models == catalog.models
            && snapshot.thinking_levels == catalog.efforts
            && snapshot.sandbox_adapter == catalog.sandbox_adapter
            && snapshot.configuration_status == ConfigurationStatus::Loaded;
        (!current).then_some(command)
    }

    pub fn refresh_snapshot_catalog(&self, snapshot: &mut RuntimeSnapshot) {
        if let Some(catalog) = self
            .catalogs
            .get(&(snapshot.harness.clone(), snapshot.project.clone()))
        {
            snapshot.models.clone_from(&catalog.models);
            snapshot.thinking_levels.clone_from(&catalog.efforts);
            if !snapshot.connected {
                snapshot
                    .sandbox_adapter
                    .clone_from(&catalog.sandbox_adapter);
            }
            snapshot.configuration_status.clone_from(&catalog.status);
        }
    }

    pub fn reconcile_snapshot(
        &mut self,
        snapshot: &mut RuntimeSnapshot,
        adopt_identity: bool,
    ) -> bool {
        let catalog = self
            .catalogs
            .entry((snapshot.harness.clone(), snapshot.project.clone()))
            .or_default();
        if snapshot.models.is_empty() || (!snapshot.connected && !catalog.models.is_empty()) {
            snapshot.models.clone_from(&catalog.models);
        } else {
            catalog.models.clone_from(&snapshot.models);
        }
        if snapshot.thinking_levels.is_empty()
            || (!snapshot.connected && !catalog.efforts.is_empty())
        {
            snapshot.thinking_levels.clone_from(&catalog.efforts);
        } else {
            catalog.efforts.clone_from(&snapshot.thinking_levels);
        }
        if !snapshot.connected && snapshot.sandbox_adapter.is_none() {
            snapshot
                .sandbox_adapter
                .clone_from(&catalog.sandbox_adapter);
        } else {
            catalog
                .sandbox_adapter
                .clone_from(&snapshot.sandbox_adapter);
        }
        snapshot.configuration_status.clone_from(&catalog.status);
        let identity = self.identities.entry(snapshot.harness.clone()).or_default();

        if let Some(session) = &snapshot.session {
            if !adopt_identity {
                return false;
            }
            let model = session.model.as_ref().map(|model| {
                snapshot
                    .models
                    .iter()
                    .find(|candidate| {
                        candidate.id == model.id && candidate.provider == model.provider
                    })
                    .cloned()
                    .unwrap_or_else(|| model.clone())
            });
            let effort = session
                .thinking_level
                .clone()
                .filter(|level| !level.is_empty())
                .filter(|_| crate::agents::supports_reasoning_effort(&snapshot.harness));
            let changed = model
                .as_ref()
                .is_some_and(|model| identity.model.as_ref() != Some(model))
                || identity.effort != effort;
            if let Some(model) = model {
                identity.model = Some(model);
            }
            identity.effort = effort;
            return changed;
        }

        if snapshot.prefill_model.is_none() {
            snapshot.prefill_model.clone_from(&identity.model);
        }
        if snapshot.prefill_thinking_level.is_none() {
            snapshot.prefill_thinking_level.clone_from(&identity.effort);
        }
        false
    }

    pub fn history_model(models: &[Model], identity: Option<&(String, String)>) -> Option<Model> {
        identity.map(|(provider, model_id)| {
            models
                .iter()
                .find(|model| model.provider == *provider && model.id == *model_id)
                .cloned()
                .unwrap_or_else(|| Model {
                    id: model_id.clone(),
                    name: model_id.clone(),
                    provider: provider.clone(),
                    context_window: 0,
                    reasoning: false,
                    resolved_model: None,
                    access_modes: None,
                    efforts: None,
                })
        })
    }
}

#[cfg(test)]
#[path = "session_identity_tests.rs"]
mod tests;
