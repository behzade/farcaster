use crate::Backend;
use std::path::{Path, PathBuf};

use farcaster_sessions::{LoadedHistory, SessionSummary, SessionTarget, SessionTransfer};

use super::{backend::for_backend, pi};

pub(super) fn validate_session_locator(harness: Backend, path: &Path) -> Result<(), String> {
    validated_locator(harness, path).map(|_| ())
}

fn validated_locator(harness: Backend, path: &Path) -> Result<Option<String>, String> {
    for_backend(harness).validate_locator(path)
}

pub fn validate_session_target(target: &SessionTarget) -> Result<(), String> {
    if validated_locator(target.harness, &target.path)?.is_some_and(|id| id != target.id) {
        return Err(format!(
            "session locator does not match its {} identity",
            target.harness
        ));
    }
    Ok(())
}

pub fn supports_session_move(harness: Backend) -> bool {
    super::known_backend_descriptors()
        .into_iter()
        .any(|backend| {
            backend.id == harness
                && backend.capabilities.sessions.move_project
                    == crate::contract::CapabilitySupport::Available
        })
}

pub fn validate_session_move(family: &[SessionSummary]) -> Result<(), String> {
    let root = family.first().ok_or("session family is empty")?;
    if !supports_session_move(root.harness) {
        return Err(format!(
            "Moving {} sessions between projects is not supported",
            root.harness
        ));
    }
    for session in family {
        validate_session_target(&session.target())?;
        if session.harness != root.harness {
            return Err("Moving a session family across harnesses is not supported".into());
        }
    }
    Ok(())
}

pub fn move_session_family(
    family: &[SessionSummary],
    target_project: &Path,
) -> Result<SessionTransfer, String> {
    validate_session_move(family)?;
    let root = &family[0];
    for_backend(root.harness).move_family(family, target_project)
}

pub fn delete_session_family(targets: &[SessionTarget]) -> Result<Vec<(PathBuf, String)>, String> {
    if targets.is_empty() {
        return Err("session family is empty".into());
    }
    for target in targets {
        validate_session_target(target)?;
        if for_backend(target.harness)
            .descriptor()
            .capabilities
            .sessions
            .delete
            != crate::contract::CapabilitySupport::Available
        {
            return Err(format!(
                "Session deletion is not supported for {}",
                target.harness
            ));
        }
    }
    let mut pi_paths = Vec::new();
    for target in targets.iter().rev() {
        if let Some(path) = for_backend(target.harness).delete_session(&target.id, &target.path)? {
            pi_paths.push(path);
        }
    }
    if pi_paths.is_empty() {
        Ok(Vec::new())
    } else {
        pi::deletion::delete_family(&pi_paths)
    }
}

pub fn load_session_history(
    harness: Backend,
    path: &Path,
    project: &Path,
) -> Result<LoadedHistory, String> {
    validate_session_locator(harness, path)?;
    for_backend(harness).load_history(path, project)
}

pub fn discover_sessions_for(
    harness: Backend,
    locator_root: Option<&Path>,
    query: &str,
) -> Result<Vec<SessionSummary>, String> {
    for_backend(harness).discover_sessions(locator_root, query)
}

pub(super) fn import_session(session: crate::DiscoveredSession) -> SessionSummary {
    let mut summary = SessionSummary::import(farcaster_sessions::SessionImport {
        id: session.id,
        harness: session.harness,
        path: session.path,
        project: session.project,
        title: session.title,
        first_user_message: session.first_user_message,
        timestamp: session.timestamp,
        parent_session: session.parent_session,
        modified: session.modified,
        message_count: session.message_count,
        usage: farcaster_sessions::UsageSummary {
            input: session.usage.input,
            output: session.usage.output,
            cache_read: session.usage.cache_read,
            cache_write: session.usage.cache_write,
            total: session.usage.total,
            cost_micros: session.usage.cost_micros,
        },
        archived: session.archived,
        is_running: session.is_running,
        search: session.search,
    });
    summary.model = session.model;
    summary.thinking_level = session.thinking_level;
    summary
}

#[cfg(test)]
#[path = "session_storage_tests.rs"]
mod tests;
