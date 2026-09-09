use std::{collections::HashMap, path::PathBuf};

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

fn effort_rank(effort: &str) -> Option<u8> {
    match effort {
        "off" | "none" => Some(0),
        "minimal" => Some(1),
        "low" => Some(2),
        "medium" => Some(3),
        "high" => Some(4),
        "xhigh" => Some(5),
        "max" => Some(6),
        _ => None,
    }
}

impl RuntimeSnapshot {
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
        let effort = self
            .session
            .as_ref()
            .and_then(|session| session.thinking_level.as_deref())
            .filter(|level| !level.is_empty())
            .or(self.prefill_thinking_level.as_deref());
        SessionIdentity {
            provider: model.map(|model| model.provider.as_str()),
            model,
            effort,
        }
    }

    pub(crate) fn available_thinking_levels(&self) -> &[String] {
        let Some(selected) = self
            .session_identity()
            .model
            .or_else(|| self.models.first())
        else {
            return &self.thinking_levels;
        };
        let model = self
            .models
            .iter()
            .find(|model| model.id == selected.id && model.provider == selected.provider)
            .unwrap_or(selected);
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

    pub fn set_catalog(
        &mut self,
        harness: String,
        project: PathBuf,
        catalog: crate::agents::ConfigurationCatalog,
    ) {
        let cached = self.catalogs.entry((harness, project)).or_default();
        cached.models = catalog.models;
        cached.efforts = catalog.efforts;
        cached.status = ConfigurationStatus::Loaded;
    }

    pub fn set_catalog_loading(&mut self, harness: String, project: PathBuf) {
        self.catalogs.entry((harness, project)).or_default().status = ConfigurationStatus::Loading;
    }

    pub fn set_catalog_error(&mut self, harness: String, project: PathBuf, error: String) {
        self.catalogs.entry((harness, project)).or_default().status =
            ConfigurationStatus::Failed(error);
    }

    pub fn refresh_snapshot_catalog(&self, snapshot: &mut RuntimeSnapshot) {
        if let Some(catalog) = self
            .catalogs
            .get(&(snapshot.harness.clone(), snapshot.project.clone()))
        {
            snapshot.models.clone_from(&catalog.models);
            snapshot.thinking_levels.clone_from(&catalog.efforts);
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
                    efforts: None,
                })
        })
    }
}

#[cfg(test)]
#[path = "session_identity_tests.rs"]
mod tests;
