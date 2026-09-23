use rmcp::schemars;
use serde::Deserialize;

use crate::agents::{CallerContext, CallerRegistry, StartWorker, WorkerContext, WorkerPool};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SendParams {
    pub to: Option<String>,
    pub message: String,
    pub profile: Option<String>,
}

pub fn send(
    pool: &WorkerPool,
    params: SendParams,
    caller_token: Option<String>,
    tasks: &crate::agents::WorkerProfiles,
    route: impl Fn(
        &crate::agents::WorkerExecution,
        &std::path::Path,
        crate::agents::HarnessAccessMode,
    ) -> Option<crate::agents::HarnessAccessMode>,
) -> Result<serde_json::Value, String> {
    send_configurable(pool, params, caller_token, tasks, route, |profile, _| {
        Err(format!(
            "worker profile '{profile}' has no available model; select one in Settings"
        ))
    })
}

pub fn send_configurable(
    pool: &WorkerPool,
    params: SendParams,
    caller_token: Option<String>,
    tasks: &crate::agents::WorkerProfiles,
    route: impl Fn(
        &crate::agents::WorkerExecution,
        &std::path::Path,
        crate::agents::HarnessAccessMode,
    ) -> Option<crate::agents::HarnessAccessMode>,
    configure: impl Fn(&str, &CallerContext) -> Result<crate::agents::WorkerExecution, String>,
) -> Result<serde_json::Value, String> {
    if params.message.trim().is_empty() {
        return Err("worker message must not be empty".into());
    }
    let token = caller_token
        .as_deref()
        .ok_or_else(|| "worker send requires a registered Farcaster caller".to_owned())?;
    let registry = CallerRegistry::shared();
    let caller = registry.resolve(token)?;
    pool.set_profile_limits(tasks)?;

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
    if let Some(assignment) = pool.queue_pending_child(
        &caller,
        &to,
        params.message.clone(),
        params.profile.as_deref(),
    )? {
        return Ok(serde_json::json!({
            "worker": to,
            "created": false,
            "queued": true,
            "pending": true,
            "assignment": assignment,
        }));
    }
    if let Some((assignment, child_access_mode)) = registry.child_assignment(&caller, &to)? {
        validate_reuse(&assignment, params.profile.as_deref())?;
        crate::agents::validate_child_access(caller.access_mode, child_access_mode)?;
        let worker = registry
            .send(token, &to, params.message.clone())?
            .ok_or("child became unavailable; retry with a new child name")?;
        return Ok(serde_json::json!({
            "worker": worker,
            "created": false,
            "queued": true,
            "pending": false,
            "assignment": assignment,
        }));
    }
    if let Some(assignment) = pool.queue_pending_child(
        &caller,
        &to,
        params.message.clone(),
        params.profile.as_deref(),
    )? {
        return Ok(serde_json::json!({
            "worker": to,
            "created": false,
            "queued": true,
            "pending": true,
            "assignment": assignment,
        }));
    }
    if let Some(assignment) = pool.resume_child(
        &caller,
        &to,
        params.message.clone(),
        params.profile.as_deref(),
        |assignment, access_mode| route(&assignment.execution, &caller.project, access_mode),
    )? {
        return Ok(serde_json::json!({
            "worker": to,
            "created": false,
            "queued": true,
            "pending": true,
            "assignment": assignment,
        }));
    }

    pool.allow_project(&caller.project)?;
    let name = to;
    let profile = params.profile.as_deref().unwrap_or("inherit");
    let requested_access_mode = delegated_access_mode(caller.backend, caller.access_mode);
    let (assignment, child_access_mode) = if profile == "inherit" {
        if !tasks.inherit_enabled {
            return Err("worker profile 'inherit' is disabled".into());
        }
        let execution = crate::agents::WorkerExecution {
            harness: caller.backend,
            provider: caller
                .provider
                .clone()
                .ok_or("cannot inherit: caller provider is unknown")?,
            model: caller
                .model
                .clone()
                .ok_or("cannot inherit: caller model is unknown")?,
            effort: caller.effort.clone(),
            service_tier: None,
        };
        execution
            .validate()
            .map_err(|error| format!("cannot inherit: {error}"))?;
        let access_mode = route(&execution, &caller.project, requested_access_mode)
            .ok_or("cannot inherit: caller model or harness is unavailable for worker creation")?;
        (
            crate::agents::WorkerAssignment {
                profile: "inherit".into(),
                execution,
            },
            access_mode,
        )
    } else {
        let definition = tasks
            .profiles
            .iter()
            .find(|definition| definition.name == profile)
            .ok_or_else(|| format!("unknown worker profile: {profile}"))?;
        if !definition.enabled {
            return Err(format!("worker profile '{profile}' is disabled"));
        }
        let selected = definition
            .models
            .first()
            .filter(|model| route(model, &caller.project, requested_access_mode).is_some())
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| configure(profile, &caller))?;
        let access_mode = route(&selected, &caller.project, requested_access_mode)
            .ok_or("selected worker model is unavailable for worker creation")?;
        (
            crate::agents::WorkerAssignment {
                profile: profile.into(),
                execution: selected,
            },
            access_mode,
        )
    };
    let initial_message = params.message;
    let concurrent_message = crate::agents::PeerMessage {
        from: caller.worker_name.clone(),
        message: initial_message.clone(),
    };
    let (assignment, created, pending) = pool.queue_assigned(
        new_worker(
            caller,
            name.clone(),
            initial_message.clone(),
            &assignment,
            child_access_mode,
        ),
        assignment,
        concurrent_message,
    )?;
    let worker = if pending {
        name
    } else {
        registry
            .send(token, &name, initial_message)?
            .ok_or("child became unavailable; retry with a new child name")?
    };
    Ok(serde_json::json!({
        "worker": worker,
        "created": created,
        "queued": true,
        "pending": pending,
        "assignment": assignment,
    }))
}

pub(super) fn delegated_access_mode(
    parent_backend: crate::agents::Backend,
    parent_access_mode: crate::agents::HarnessAccessMode,
) -> crate::agents::HarnessAccessMode {
    match (parent_backend, parent_access_mode) {
        // Pi's mode describes parent containment, not how autonomous children
        // should handle approvals.
        (crate::agents::Backend::Pi, crate::agents::HarnessAccessMode::Sandboxed) => {
            crate::agents::HarnessAccessMode::Auto
        }
        (_, access_mode) => access_mode,
    }
}

#[cfg(test)]
fn resolve_child(
    profiles: &crate::agents::WorkerProfiles,
    profile: &str,
    project: &std::path::Path,
    parent_access_mode: crate::agents::HarnessAccessMode,
    route: impl Fn(
        &crate::agents::WorkerExecution,
        &std::path::Path,
        crate::agents::HarnessAccessMode,
    ) -> Option<crate::agents::HarnessAccessMode>,
) -> Result<
    (
        crate::agents::WorkerAssignment,
        crate::agents::HarnessAccessMode,
    ),
    String,
> {
    let assignment = profiles.resolve(profile, |model| {
        route(model, project, parent_access_mode).is_some()
    })?;
    let access_mode = route(&assignment.execution, project, parent_access_mode)
        .ok_or("selected worker model no longer supports the required child access mode")?;
    Ok((assignment, access_mode))
}

pub(super) fn child_access_mode(
    model: &crate::agents::WorkerExecution,
    project: &std::path::Path,
    parent_access_mode: crate::agents::HarnessAccessMode,
    backends: &[crate::agents::Backend],
    catalogs: &[crate::storage::CachedConfigurationCatalog],
) -> Option<crate::agents::HarnessAccessMode> {
    if !model_available(model, project, backends, catalogs) {
        return None;
    }
    if parent_access_mode == crate::agents::HarnessAccessMode::Full {
        return Some(parent_access_mode);
    }
    let catalogs_for_harness = catalogs
        .iter()
        .filter(|entry| entry.harness == model.harness && entry.project == project)
        .collect::<Vec<_>>();
    let catalog_model = catalogs_for_harness.iter().find_map(|entry| {
        entry
            .catalog
            .models
            .iter()
            .find(|candidate| candidate.provider == model.provider && candidate.id == model.model)
            .map(|candidate| (*entry, candidate))
    });
    if model.harness == crate::agents::Backend::Pi
        && catalog_model
            .and_then(|(entry, _)| entry.catalog.sandbox_adapter.as_deref())
            .is_none()
    {
        return None;
    }
    let modes = match catalog_model {
        Some((entry, candidate)) => crate::agents::available_access_modes(
            model.harness,
            Some(candidate),
            entry.catalog.sandbox_adapter.as_deref(),
        ),
        None if catalogs_for_harness.is_empty() => {
            crate::agents::available_access_modes(model.harness, None, None)
        }
        None => Vec::new(),
    };
    match parent_access_mode {
        crate::agents::HarnessAccessMode::Auto
            if modes.contains(&crate::agents::HarnessAccessMode::Auto) =>
        {
            Some(crate::agents::HarnessAccessMode::Auto)
        }
        crate::agents::HarnessAccessMode::Auto | crate::agents::HarnessAccessMode::Sandboxed
            if modes.contains(&crate::agents::HarnessAccessMode::Sandboxed) =>
        {
            Some(crate::agents::HarnessAccessMode::Sandboxed)
        }
        crate::agents::HarnessAccessMode::Auto
        | crate::agents::HarnessAccessMode::Sandboxed
        | crate::agents::HarnessAccessMode::Full => None,
    }
}

pub(super) fn model_available(
    model: &crate::agents::WorkerExecution,
    project: &std::path::Path,
    backends: &[crate::agents::Backend],
    catalogs: &[crate::storage::CachedConfigurationCatalog],
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
    access_mode: crate::agents::HarnessAccessMode,
) -> StartWorker {
    StartWorker {
        project: caller.project,
        name: name.clone(),
        prompt: format!(
            "Task delegated by Farcaster parent {} to child {name}:\n\n{message}",
            caller.worker_name
        ),
        backend: assignment.execution.harness,
        parent_session: caller.session,
        parent_worker_id: Some(caller.worker_id),
        context: WorkerContext::Fresh,
        provider: Some(assignment.execution.provider.clone()),
        model: Some(assignment.execution.model.clone()),
        effort: assignment.execution.effort.clone(),
        access_mode,
    }
}

#[cfg(test)]
#[path = "workers_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workers_restart_tests.rs"]
mod restart_tests;
