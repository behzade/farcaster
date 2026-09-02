use std::{collections::HashMap, path::PathBuf};

use crate::protocol::Model;

use super::RuntimeSnapshot;

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
    pub(crate) fn session_identity(&self) -> SessionIdentity<'_> {
        let model = self
            .session
            .as_ref()
            .and_then(|session| session.model.as_ref())
            .or(self.prefill_model.as_ref());
        let effort = self
            .session
            .as_ref()
            .map(|session| session.thinking_level.as_str())
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
pub(super) struct SessionControlDefaults {
    identities: HashMap<String, OwnedSessionIdentity>,
    catalogs: HashMap<(String, PathBuf), HarnessCatalog>,
}

#[derive(Default)]
struct HarnessCatalog {
    models: Vec<Model>,
    efforts: Vec<String>,
}

#[derive(Default)]
struct OwnedSessionIdentity {
    model: Option<Model>,
    effort: Option<String>,
}

impl SessionControlDefaults {
    pub fn restore(
        &mut self,
        entries: Vec<crate::app::infrastructure::persistence::CachedSessionControlDefaults>,
    ) {
        for entry in entries {
            self.identities.insert(
                entry.harness,
                OwnedSessionIdentity {
                    model: entry.model,
                    effort: entry.effort,
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

    pub fn select_model(&mut self, harness: &str, model: Model) -> bool {
        let identity = self.identities.entry(harness.to_owned()).or_default();
        let replacement = replacement_effort(&model, identity.effort.as_deref());
        let changed = identity.model.as_ref() != Some(&model) || replacement.is_some();
        identity.model = Some(model);
        if let Some(effort) = replacement {
            identity.effort = Some(effort);
        }
        changed
    }

    pub fn select_effort(&mut self, harness: &str, effort: String) -> bool {
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
    }

    pub fn apply(&mut self, snapshot: &mut RuntimeSnapshot, adopt_identity: bool) -> bool {
        let catalog = self
            .catalogs
            .entry((snapshot.harness.clone(), snapshot.project.clone()))
            .or_default();
        if snapshot.models.is_empty() {
            snapshot.models.clone_from(&catalog.models);
        } else {
            catalog.models.clone_from(&snapshot.models);
        }
        if snapshot.thinking_levels.is_empty() {
            snapshot.thinking_levels.clone_from(&catalog.efforts);
        } else {
            catalog.efforts.clone_from(&snapshot.thinking_levels);
        }
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
            let changed = model
                .as_ref()
                .is_some_and(|model| identity.model.as_ref() != Some(model))
                || identity.effort.as_deref() != Some(session.thinking_level.as_str());
            if let Some(model) = model {
                identity.model = Some(model);
            }
            identity.effort = Some(session.thinking_level.clone());
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
mod tests {
    use super::*;

    fn model(id: &str, reasoning: bool, efforts: Option<&[&str]>) -> Model {
        Model {
            id: id.into(),
            name: id.into(),
            provider: "provider".into(),
            context_window: 0,
            reasoning,
            efforts: efforts.map(|efforts| efforts.iter().map(|effort| (*effort).into()).collect()),
        }
    }

    #[test]
    fn available_thinking_levels_follow_the_selected_model() {
        let selected = model("selected", true, Some(&["low", "medium"]));
        let snapshot = RuntimeSnapshot {
            prefill_model: Some(selected.clone()),
            models: vec![selected, model("other", true, Some(&["high", "xhigh"]))],
            thinking_levels: vec!["low".into(), "medium".into(), "high".into(), "xhigh".into()],
            ..RuntimeSnapshot::default()
        };

        assert_eq!(snapshot.available_thinking_levels(), ["low", "medium"]);
    }

    #[test]
    fn available_thinking_levels_use_the_first_model_for_a_new_draft() {
        let snapshot = RuntimeSnapshot {
            models: vec![model("default", true, Some(&["minimal", "low"]))],
            thinking_levels: vec!["minimal".into(), "low".into(), "high".into()],
            ..RuntimeSnapshot::default()
        };

        assert_eq!(snapshot.available_thinking_levels(), ["minimal", "low"]);
    }

    #[test]
    fn available_thinking_levels_keep_legacy_global_catalogs() {
        let snapshot = RuntimeSnapshot {
            prefill_model: Some(model("legacy", true, None)),
            thinking_levels: vec!["off".into(), "high".into()],
            ..RuntimeSnapshot::default()
        };

        assert_eq!(snapshot.available_thinking_levels(), ["off", "high"]);
    }

    #[test]
    fn known_empty_model_efforts_do_not_fall_back_to_other_models() {
        let snapshot = RuntimeSnapshot {
            prefill_model: Some(model("fixed", true, Some(&[]))),
            thinking_levels: vec!["low".into(), "high".into()],
            ..RuntimeSnapshot::default()
        };

        assert!(snapshot.available_thinking_levels().is_empty());
    }

    #[test]
    fn non_reasoning_models_have_no_effort_choices() {
        let snapshot = RuntimeSnapshot {
            prefill_model: Some(model("plain", false, None)),
            thinking_levels: vec!["off".into(), "high".into()],
            ..RuntimeSnapshot::default()
        };

        assert!(snapshot.available_thinking_levels().is_empty());
    }

    #[test]
    fn selecting_model_replaces_an_unsupported_cached_effort_with_the_nearest_level() {
        let mut defaults = SessionControlDefaults::default();
        assert!(defaults.select_effort("pi", "high".into()));
        assert!(defaults.select_model(
            "pi",
            model("limited", true, Some(&["off", "low", "medium"])),
        ));

        let mut draft = RuntimeSnapshot {
            harness: "pi".into(),
            ..RuntimeSnapshot::default()
        };
        defaults.apply(&mut draft, true);

        assert_eq!(draft.prefill_thinking_level.as_deref(), Some("medium"));
    }

    #[test]
    fn cached_defaults_restore_across_projects_per_harness() {
        let selected = model("selected", true, Some(&["low", "high"]));
        let mut defaults = SessionControlDefaults::default();
        assert!(defaults.select_model("codex-cli", selected.clone()));
        assert!(defaults.select_effort("codex-cli", "high".into()));

        let mut restarted = SessionControlDefaults::default();
        restarted.restore(defaults.cached());
        let mut draft = RuntimeSnapshot {
            harness: "codex-cli".into(),
            project: PathBuf::from("/another-project"),
            ..RuntimeSnapshot::default()
        };
        restarted.apply(&mut draft, true);

        assert_eq!(draft.prefill_model, Some(selected));
        assert_eq!(draft.prefill_thinking_level.as_deref(), Some("high"));

        let mut other_harness = RuntimeSnapshot {
            harness: "pi".into(),
            project: PathBuf::from("/another-project"),
            ..RuntimeSnapshot::default()
        };
        restarted.apply(&mut other_harness, true);
        assert_eq!(other_harness.prefill_model, None);
        assert_eq!(other_harness.prefill_thinking_level, None);
    }
}
