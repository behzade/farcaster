use std::{collections::HashMap, path::PathBuf};

use crate::protocol::Model;

use super::RuntimeSnapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SessionIdentity<'a> {
    pub provider: Option<&'a str>,
    pub model: Option<&'a Model>,
    pub effort: Option<&'a str>,
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
    by_target: HashMap<(String, PathBuf), HarnessDefaults>,
}

#[derive(Default)]
struct HarnessDefaults {
    identity: OwnedSessionIdentity,
    models: Vec<Model>,
    efforts: Vec<String>,
}

#[derive(Default)]
struct OwnedSessionIdentity {
    model: Option<Model>,
    effort: Option<String>,
}

impl SessionControlDefaults {
    pub fn set_catalog(
        &mut self,
        harness: String,
        project: PathBuf,
        catalog: crate::agents::ConfigurationCatalog,
    ) {
        let defaults = self.by_target.entry((harness, project)).or_default();
        defaults.models = catalog.models;
        defaults.efforts = catalog.efforts;
    }

    pub fn apply(&mut self, snapshot: &mut RuntimeSnapshot, adopt_identity: bool) {
        let defaults = self
            .by_target
            .entry((snapshot.harness.clone(), snapshot.project.clone()))
            .or_default();
        if snapshot.models.is_empty() {
            snapshot.models.clone_from(&defaults.models);
        } else {
            defaults.models.clone_from(&snapshot.models);
        }
        if snapshot.thinking_levels.is_empty() {
            snapshot.thinking_levels.clone_from(&defaults.efforts);
        } else {
            defaults.efforts.clone_from(&snapshot.thinking_levels);
        }

        if let Some(session) = &snapshot.session {
            if adopt_identity {
                if let Some(model) = &session.model {
                    defaults.identity.model = snapshot
                        .models
                        .iter()
                        .find(|candidate| {
                            candidate.id == model.id && candidate.provider == model.provider
                        })
                        .cloned()
                        .or_else(|| Some(model.clone()));
                }
                defaults.identity.effort = Some(session.thinking_level.clone());
            }
            return;
        }

        if snapshot.prefill_model.is_none() {
            snapshot.prefill_model.clone_from(&defaults.identity.model);
        }
        if snapshot.prefill_thinking_level.is_none() {
            snapshot
                .prefill_thinking_level
                .clone_from(&defaults.identity.effort);
        }
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
}
