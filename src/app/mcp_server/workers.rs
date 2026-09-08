use rmcp::schemars;
use serde::Deserialize;

use crate::agents::{CallerContext, CallerRegistry, StartWorker, WorkerContext, WorkerPool};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SendParams {
    pub(super) to: Option<String>,
    pub(super) message: String,
    pub(super) profile: Option<String>,
}

pub(super) fn send(
    pool: &WorkerPool,
    params: SendParams,
    caller_token: Option<String>,
    tasks: &crate::agents::WorkerProfiles,
    available: impl Fn(&crate::agents::WorkerExecution, &std::path::Path) -> bool,
) -> Result<serde_json::Value, String> {
    if params.message.trim().is_empty() {
        return Err("worker message must not be empty".into());
    }
    let token = caller_token
        .as_deref()
        .ok_or_else(|| "worker send requires a registered Farcaster caller".to_owned())?;
    let registry = CallerRegistry::shared();
    let caller = registry.resolve(token)?;

    if caller.parent_worker_id.is_some() {
        if params.profile.is_some() {
            return Err("children cannot select worker profiles".into());
        }
        let worker = registry
            .send(token, "", params.message)?
            .ok_or_else(|| "parent worker is unavailable".to_owned())?;
        return Ok(serde_json::json!({
            "worker": worker,
            "created": false,
            "queued": true,
        }));
    }

    let to = params
        .to
        .ok_or_else(|| "top-level workers must provide a child name in `to`".to_owned())?;
    if !crate::agents::valid_worker_name(&to) {
        return Err("child name must be 1-48 ASCII letters, numbers, '-' or '_' and cannot start with punctuation".into());
    }
    if let Some(assignment) = registry.child_assignment(&caller, &to)? {
        validate_reuse(&assignment, params.profile.as_deref())?;
        let worker = registry
            .send(token, &to, params.message.clone())?
            .ok_or("child became unavailable; retry with a new child name")?;
        return Ok(serde_json::json!({
            "worker": worker,
            "created": false,
            "queued": true,
            "assignment": assignment,
        }));
    }

    pool.allow_project(&caller.project)?;
    let name = to;
    let profile = params.profile.as_deref().ok_or(
        "new children require a configured `profile`; omit profile only when reusing a child",
    )?;
    let assignment = tasks.resolve(profile, |model| available(model, &caller.project))?;
    pool.start_assigned(
        new_worker(caller, name.clone(), params.message, &assignment),
        Some(assignment.clone()),
    )?;
    Ok(serde_json::json!({
        "worker": name,
        "created": true,
        "queued": true,
        "assignment": assignment,
    }))
}

pub(super) fn model_available(
    model: &crate::agents::WorkerExecution,
    project: &std::path::Path,
    backends: &[String],
    catalogs: &[crate::app::persistence::CachedConfigurationCatalog],
) -> bool {
    if !backends.contains(&model.harness) {
        return false;
    }
    let mut catalogs = catalogs
        .iter()
        .filter(|entry| entry.harness == model.harness && entry.project == project)
        .peekable();
    // Without a catalog, allow the installed harness to validate the configured IDs.
    // With a catalog, skip providers and models that this harness does not offer.
    catalogs.peek().is_none()
        || catalogs.any(|entry| {
            entry.catalog.models.iter().any(|candidate| {
                candidate.provider == model.provider && candidate.id == model.model
            })
        })
}

fn validate_reuse(
    assignment: &crate::agents::WorkerAssignment,
    profile: Option<&str>,
) -> Result<(), String> {
    if profile.is_some_and(|profile| profile != assignment.profile) {
        return Err(
            "a child's profile is fixed at creation; use a new child name for a different profile"
                .into(),
        );
    }
    Ok(())
}

fn new_worker(
    caller: CallerContext,
    name: String,
    message: String,
    assignment: &crate::agents::WorkerAssignment,
) -> StartWorker {
    StartWorker {
        project: caller.project,
        name: name.clone(),
        prompt: format!(
            "Task delegated by Farcaster parent {} to child {name}:\n\n{message}",
            caller.worker_name
        ),
        backend: assignment.execution.harness.clone(),
        parent_session: caller.session,
        parent_worker_id: Some(caller.worker_id),
        context: WorkerContext::Fresh,
        provider: Some(assignment.execution.provider.clone()),
        model: Some(assignment.execution.model.clone()),
        effort: assignment.execution.effort.clone(),
    }
}

#[cfg(test)]
#[path = "workers_tests.rs"]
mod tests;
