use crate::agents::Backend;
use std::{collections::HashMap, path::PathBuf, sync::LazyLock};

use crate::agents::effort_rank;
use crate::protocol::Model;

use super::{ConfigurationStatus, RuntimeSnapshot};

static STANDARD_SERVICE_TIER: LazyLock<Vec<String>> = LazyLock::new(|| vec!["standard".to_owned()]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionIdentity<'a> {
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
    pub fn available_access_modes(&self) -> Vec<crate::agents::HarnessAccessMode> {
        crate::agents::available_access_modes(
            self.harness,
            self.access_mode_model(),
            self.sandbox_adapter.as_deref(),
        )
    }

    pub fn sandbox_controls_available(&self) -> bool {
        !crate::agents::supports_sandbox_discovery(self.harness) || self.sandbox_adapter.is_some()
    }

    pub fn access_mode_for_new_session(&self) -> crate::agents::HarnessAccessMode {
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

    pub fn catalog_model<'a>(&'a self, selected: &'a Model) -> &'a Model {
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

    pub fn session_target(&self) -> Option<crate::sessions::SessionTarget> {
        let state = self.session.as_ref()?;
        if self.harness.is_none() || state.session_id.is_empty() {
            return None;
        }
        Some(crate::sessions::SessionTarget {
            harness: self.harness?,
            id: state.session_id.clone(),
            path: crate::sessions::normalize_session_path(self.selected_session.as_deref()?),
        })
    }

    pub fn session_identity(&self) -> SessionIdentity<'_> {
        let model = if self.pending_initial_model {
            self.prefill_model.as_ref()
        } else {
            self.session
                .as_ref()
                .and_then(|session| session.model.as_ref())
                .or(self.prefill_model.as_ref())
        };
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

    pub fn selected_service_tier(&self) -> Option<&str> {
        if self.pending_initial_service_tier {
            self.prefill_service_tier.as_deref()
        } else {
            match &self.session {
                Some(session) => session.service_tier.as_deref(),
                None => self.prefill_service_tier.as_deref(),
            }
        }
    }

    pub fn available_service_tiers(&self) -> &[String] {
        if let Some(selected) = self.session_identity().model {
            let tiers = self.catalog_model(selected).service_tiers.as_slice();
            if !tiers.is_empty() {
                return tiers;
            }
            if self.pending_initial_model {
                return self.fallback_service_tiers();
            }
            if let Some(session) = &self.session
                && session.model.as_ref().is_some_and(|model| {
                    model.provider == selected.provider && model.id == selected.id
                })
                && !session.service_tiers.is_empty()
            {
                return &session.service_tiers;
            }
            return self.fallback_service_tiers();
        }
        let tiers = self
            .session
            .as_ref()
            .map(|session| session.service_tiers.as_slice())
            .or_else(|| {
                self.models
                    .first()
                    .map(|model| model.service_tiers.as_slice())
            })
            .unwrap_or(&[]);
        if tiers.is_empty() {
            self.fallback_service_tiers()
        } else {
            tiers
        }
    }

    fn fallback_service_tiers(&self) -> &[String] {
        if matches!(self.harness, Some(Backend::Codex | Backend::Claude)) {
            &STANDARD_SERVICE_TIER
        } else {
            &[]
        }
    }

    pub fn effort_choices(&self, model: &Model) -> Vec<Option<String>> {
        crate::agents::supports_reasoning_reset(self.harness)
            .then_some(None)
            .into_iter()
            .chain(
                crate::agents::model_efforts(self.catalog_model(model), &self.thinking_levels)
                    .into_iter()
                    .map(Some),
            )
            .collect()
    }

    pub fn available_thinking_levels(&self) -> &[String] {
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
    identities: HashMap<(Backend, Option<String>), OwnedSessionIdentity>,
    catalogs: HashMap<(Backend, Option<String>, PathBuf), HarnessCatalog>,
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
    access_mode: Option<crate::agents::HarnessAccessMode>,
}

impl HarnessConfigurationStore {
    pub fn restore(&mut self, entries: Vec<farcaster_storage::CachedSessionControlDefaults>) {
        for entry in entries {
            let effort = entry
                .effort
                .filter(|_| crate::agents::supports_reasoning_effort(entry.harness));
            self.identities.insert(
                (entry.harness, entry.profile_id),
                OwnedSessionIdentity {
                    model: entry.model,
                    effort,
                    access_mode: entry.access_mode,
                },
            );
        }
    }

    pub fn cached(&self) -> Vec<farcaster_storage::CachedSessionControlDefaults> {
        let mut entries = self
            .identities
            .iter()
            .filter(|(_, identity)| {
                identity.model.is_some()
                    || identity.effort.is_some()
                    || identity.access_mode.is_some()
            })
            .map(|((harness, profile_id), identity)| {
                farcaster_storage::CachedSessionControlDefaults {
                    harness: *harness,
                    profile_id: profile_id.clone(),
                    model: identity.model.clone(),
                    effort: identity.effort.clone(),
                    access_mode: identity.access_mode,
                }
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| (a.harness, &a.profile_id).cmp(&(b.harness, &b.profile_id)));
        entries
    }

    #[cfg(test)]
    pub fn model(&self, harness: impl Into<Option<Backend>>) -> Option<&Model> {
        self.model_for(harness, None)
    }

    pub fn model_for(
        &self,
        harness: impl Into<Option<Backend>>,
        profile_id: Option<&str>,
    ) -> Option<&Model> {
        let harness = harness.into()?;
        self.identities
            .get(&(harness, profile_id.map(str::to_owned)))
            .and_then(|identity| identity.model.as_ref())
    }

    #[cfg(test)]
    pub fn effort(&self, harness: impl Into<Option<Backend>>) -> Option<&str> {
        self.effort_for(harness, None)
    }

    pub fn effort_for(
        &self,
        harness: impl Into<Option<Backend>>,
        profile_id: Option<&str>,
    ) -> Option<&str> {
        let harness = harness.into()?;
        self.identities
            .get(&(harness, profile_id.map(str::to_owned)))
            .and_then(|identity| identity.effort.as_deref())
    }

    #[cfg(test)]
    pub fn access_mode(
        &self,
        harness: impl Into<Option<Backend>>,
    ) -> Option<crate::agents::HarnessAccessMode> {
        self.access_mode_for(harness, None)
    }

    pub fn access_mode_for(
        &self,
        harness: impl Into<Option<Backend>>,
        profile_id: Option<&str>,
    ) -> Option<crate::agents::HarnessAccessMode> {
        let harness = harness.into()?;
        self.identities
            .get(&(harness, profile_id.map(str::to_owned)))
            .and_then(|identity| identity.access_mode)
    }

    #[cfg(test)]
    pub fn set_model(&mut self, harness: impl Into<Option<Backend>>, model: Model) -> bool {
        self.set_model_for(harness, None, model)
    }

    pub fn set_model_for(
        &mut self,
        harness: impl Into<Option<Backend>>,
        profile_id: Option<&str>,
        model: Model,
    ) -> bool {
        let Some(harness) = harness.into() else {
            return false;
        };
        let identity = self
            .identities
            .entry((harness, profile_id.map(str::to_owned)))
            .or_default();
        let replacement = replacement_effort(&model, identity.effort.as_deref());
        let changed = identity.model.as_ref() != Some(&model) || replacement.is_some();
        identity.model = Some(model);
        if let Some(effort) = replacement {
            identity.effort = Some(effort);
        }
        changed
    }

    #[cfg(test)]
    pub fn set_effort(&mut self, harness: impl Into<Option<Backend>>, effort: String) -> bool {
        self.set_effort_for(harness, None, effort)
    }

    pub fn set_effort_for(
        &mut self,
        harness: impl Into<Option<Backend>>,
        profile_id: Option<&str>,
        effort: String,
    ) -> bool {
        let Some(harness) = harness.into() else {
            return false;
        };
        if !crate::agents::supports_reasoning_effort(harness) {
            return false;
        }
        let identity = self
            .identities
            .entry((harness, profile_id.map(str::to_owned)))
            .or_default();
        if identity.effort.as_ref() == Some(&effort) {
            return false;
        }
        identity.effort = Some(effort);
        true
    }

    #[cfg(test)]
    pub fn reset_effort(&mut self, harness: impl Into<Option<Backend>>) -> bool {
        self.reset_effort_for(harness, None)
    }

    pub fn reset_effort_for(
        &mut self,
        harness: impl Into<Option<Backend>>,
        profile_id: Option<&str>,
    ) -> bool {
        let Some(harness) = harness.into() else {
            return false;
        };
        if !crate::agents::supports_reasoning_reset(harness) {
            return false;
        }
        self.identities
            .entry((harness, profile_id.map(str::to_owned)))
            .or_default()
            .effort
            .take()
            .is_some()
    }

    #[cfg(test)]
    pub fn set_access_mode(
        &mut self,
        harness: impl Into<Option<Backend>>,
        access_mode: crate::agents::HarnessAccessMode,
    ) -> bool {
        self.set_access_mode_for(harness, None, access_mode)
    }

    pub fn set_access_mode_for(
        &mut self,
        harness: impl Into<Option<Backend>>,
        profile_id: Option<&str>,
        access_mode: crate::agents::HarnessAccessMode,
    ) -> bool {
        let Some(harness) = harness.into() else {
            return false;
        };
        let identity = self
            .identities
            .entry((harness, profile_id.map(str::to_owned)))
            .or_default();
        if identity.access_mode == Some(access_mode) {
            return false;
        }
        identity.access_mode = Some(access_mode);
        true
    }

    #[cfg(test)]
    pub fn set_catalog(
        &mut self,
        harness: Backend,
        project: PathBuf,
        catalog: crate::agents::ConfigurationCatalog,
    ) {
        self.set_catalog_for_profile(harness, None, project, catalog);
    }

    pub fn set_catalog_for_profile(
        &mut self,
        harness: Backend,
        profile_id: Option<String>,
        project: PathBuf,
        catalog: crate::agents::ConfigurationCatalog,
    ) {
        let cached = self
            .catalogs
            .entry((harness, profile_id, project))
            .or_default();
        cached.models = catalog.models;
        cached.efforts = catalog.efforts;
        cached.sandbox_adapter = catalog.sandbox_adapter;
        cached.status = ConfigurationStatus::Loaded;
    }

    pub fn set_catalog_loading(
        &mut self,
        harness: Backend,
        profile_id: Option<String>,
        project: PathBuf,
    ) {
        self.catalogs
            .entry((harness, profile_id, project))
            .or_default()
            .status = ConfigurationStatus::Loading;
    }

    pub fn set_catalog_error(
        &mut self,
        harness: Backend,
        profile_id: Option<String>,
        project: PathBuf,
        error: String,
    ) {
        self.catalogs
            .entry((harness, profile_id, project))
            .or_default()
            .status = ConfigurationStatus::Failed(error);
    }

    pub fn catalog_command(
        &self,
        harness: impl Into<Option<Backend>>,
        project: &std::path::Path,
    ) -> Option<super::RuntimeCommand> {
        self.catalog_command_for_profile(harness, None, project)
    }

    pub fn catalog_command_for_profile(
        &self,
        harness: impl Into<Option<Backend>>,
        profile_id: Option<&str>,
        project: &std::path::Path,
    ) -> Option<super::RuntimeCommand> {
        let harness = harness.into()?;
        let catalog = self.catalogs.get(&(
            harness.to_owned(),
            profile_id.map(str::to_owned),
            project.to_owned(),
        ))?;
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
        snapshot.harness?;
        if snapshot.connected {
            return None;
        }
        let command = self.catalog_command_for_profile(
            snapshot.harness,
            snapshot.profile_id.as_deref(),
            &snapshot.project,
        )?;
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
        let Some(harness) = snapshot.harness else {
            return;
        };
        if let Some(catalog) = self.catalogs.get(&(
            harness,
            snapshot.profile_id.clone(),
            snapshot.project.clone(),
        )) {
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
        let Some(harness) = snapshot.harness else {
            return false;
        };
        let catalog = self
            .catalogs
            .entry((
                harness,
                snapshot.profile_id.clone(),
                snapshot.project.clone(),
            ))
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
        let identity = self
            .identities
            .entry((harness, snapshot.profile_id.clone()))
            .or_default();

        if let Some(session) = &snapshot.session {
            if !adopt_identity {
                return false;
            }
            if snapshot.pending_initial_model {
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
                .filter(|_| crate::agents::supports_reasoning_effort(snapshot.harness));
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
                    service_tiers: Vec::new(),
                    access_modes: None,
                    efforts: None,
                })
        })
    }
}

#[cfg(test)]
#[path = "session_identity_tests.rs"]
mod tests;
