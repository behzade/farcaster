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
}

#[derive(Default)]
pub(super) struct SessionControlDefaults {
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
    /// Apply shared control defaults to `snapshot`. `adopt_identity` controls whether a
    /// live session's model and effort become the default for future new sessions; it
    /// must be false for background sessions and subagents the user is merely viewing.
    pub fn apply(&mut self, snapshot: &mut RuntimeSnapshot, adopt_identity: bool) {
        if snapshot.models.is_empty() {
            snapshot.models.clone_from(&self.models);
        } else {
            self.models.clone_from(&snapshot.models);
        }
        if snapshot.thinking_levels.is_empty() {
            snapshot.thinking_levels.clone_from(&self.efforts);
        } else {
            self.efforts.clone_from(&snapshot.thinking_levels);
        }

        if let Some(session) = &snapshot.session {
            if adopt_identity {
                if let Some(model) = &session.model {
                    self.identity.model = self
                        .models
                        .iter()
                        .find(|candidate| {
                            candidate.id == model.id && candidate.provider == model.provider
                        })
                        .cloned()
                        .or_else(|| Some(model.clone()));
                }
                self.identity.effort = Some(session.thinking_level.clone());
            }
            return;
        }

        if snapshot.prefill_model.is_none() {
            snapshot.prefill_model.clone_from(&self.identity.model);
        }
        if snapshot.prefill_thinking_level.is_none() {
            snapshot
                .prefill_thinking_level
                .clone_from(&self.identity.effort);
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
                })
        })
    }
}
