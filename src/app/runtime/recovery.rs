use std::{collections::HashMap, path::Path};

use super::*;

pub(super) const DISCARD_INTERRUPTED_PROMPT: &str = "Discard";
pub(super) const MARK_INTERRUPTED_PROMPT_DELIVERED: &str = "Mark delivered";
const RECOVERY_DIALOG_PREFIX: &str = "farcaster-recovery-";

pub(in crate::app) fn is_recovery_dialog(request: &ExtensionUiRequest) -> bool {
    request.dialog_id().is_some_and(is_recovery_dialog_id)
}

pub(in crate::app) fn is_recovery_dialog_id(id: &str) -> bool {
    id.starts_with(RECOVERY_DIALOG_PREFIX)
}

#[derive(Default)]
pub(super) struct InterruptedPromptRecovery {
    prompts: HashMap<i64, crate::agents::QueuedPrompt>,
}

pub(super) enum RecoveryResolution {
    NotRecovery,
    Pending,
    Resolved {
        target: String,
        session: Option<std::path::PathBuf>,
    },
}

impl InterruptedPromptRecovery {
    pub(super) fn recover(state: &StateStore) -> Result<Self, String> {
        Ok(Self {
            prompts: state
                .recover_interrupted_prompts()?
                .into_iter()
                .map(|prompt| (prompt.id, prompt))
                .collect(),
        })
    }

    pub(super) fn prompts(&self) -> impl Iterator<Item = &crate::agents::QueuedPrompt> {
        self.prompts.values()
    }

    pub(super) fn target_for(
        &self,
        target: &str,
        project: &Path,
        session: Option<&Path>,
    ) -> Option<&str> {
        self.prompts
            .values()
            .find(|prompt| belongs_to_selection(prompt, target, project, session))
            .map(|prompt| prompt.target.as_str())
    }

    pub(super) fn requests_for(
        &self,
        target: &str,
        project: &Path,
        session: Option<&Path>,
    ) -> Vec<ExtensionUiRequest> {
        let mut prompts = self
            .prompts
            .values()
            .filter(|prompt| belongs_to_selection(prompt, target, project, session))
            .collect::<Vec<_>>();
        prompts.sort_by_key(|prompt| prompt.id);
        prompts.into_iter().map(recovery_request).collect()
    }

    pub(super) fn resolve(
        &mut self,
        state: &mut StateStore,
        target: &str,
        project: &Path,
        session: Option<&Path>,
        response: &ExtensionUiResponse,
    ) -> Result<RecoveryResolution, String> {
        let (id, action) = match response {
            ExtensionUiResponse::Value { id, value } => {
                let Some(id) = recovery_id(id)? else {
                    return Ok(RecoveryResolution::NotRecovery);
                };
                (id, Some(value.as_str()))
            }
            ExtensionUiResponse::Cancelled { id, .. }
            | ExtensionUiResponse::Confirmed { id, .. } => {
                let Some(id) = recovery_id(id)? else {
                    return Ok(RecoveryResolution::NotRecovery);
                };
                (id, None)
            }
        };
        let expected = self
            .prompts
            .get(&id)
            .ok_or_else(|| format!("interrupted prompt {id} is no longer awaiting disposition"))?;
        if !belongs_to_selection(expected, target, project, session) {
            return Err(format!(
                "interrupted prompt {id} does not belong to the selected session and project"
            ));
        }
        let Some(action) = action else {
            return Ok(RecoveryResolution::Pending);
        };
        let current = state
            .unknown_prompts()?
            .into_iter()
            .find(|prompt| prompt.id == id)
            .ok_or_else(|| format!("interrupted prompt {id} is no longer awaiting disposition"))?;
        if &current != expected {
            return Err(format!(
                "interrupted prompt {id} changed before its disposition was applied"
            ));
        }
        match action {
            DISCARD_INTERRUPTED_PROMPT => {
                state.discard_unknown_prompt(id, &current.target, current.session.as_deref())?
            }
            MARK_INTERRUPTED_PROMPT_DELIVERED => {
                state.reconcile_unknown_prompt(id, &current.target, current.session.as_deref())?
            }
            _ => return Err(format!("unknown interrupted prompt action: {action}")),
        }
        self.prompts.remove(&id);
        Ok(RecoveryResolution::Resolved {
            target: current.target,
            session: current.session,
        })
    }
}

fn recovery_id(id: &str) -> Result<Option<i64>, String> {
    let Some(id) = id.strip_prefix(RECOVERY_DIALOG_PREFIX) else {
        return Ok(None);
    };
    id.parse()
        .map(Some)
        .map_err(|_| format!("invalid interrupted prompt recovery id: {id}"))
}

fn belongs_to_selection(
    prompt: &crate::agents::QueuedPrompt,
    target: &str,
    project: &Path,
    session: Option<&Path>,
) -> bool {
    crate::sessions::normalize_session_path(&prompt.project)
        == crate::sessions::normalize_session_path(project)
        && (prompt.target == target
            || prompt.session.as_deref().is_some_and(|prompt_session| {
                session.is_some_and(|session| {
                    crate::sessions::normalize_session_path(prompt_session)
                        == crate::sessions::normalize_session_path(session)
                })
            }))
}

fn recovery_request(prompt: &crate::agents::QueuedPrompt) -> ExtensionUiRequest {
    ExtensionUiRequest::Select {
        id: format!("{RECOVERY_DIALOG_PREFIX}{}", prompt.id),
        title: format!(
            "Was this prompt delivered before Farcaster stopped?\n\n{}",
            prompt.message
        ),
        options: vec![
            MARK_INTERRUPTED_PROMPT_DELIVERED.to_owned(),
            DISCARD_INTERRUPTED_PROMPT.to_owned(),
        ],
        timeout: None,
    }
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
