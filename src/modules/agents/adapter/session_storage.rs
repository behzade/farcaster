use crate::agents::Backend;
use std::path::{Path, PathBuf};

use crate::sessions::{
    LoadedHistory, SessionSummary, SessionTarget, SessionTransfer, TransferMember,
};

use super::{codex, cursor, main_session::external_session_locator, opencode, pi};

pub(super) fn validate_session_locator(harness: Backend, path: &Path) -> Result<(), String> {
    validated_locator(harness, path).map(|_| ())
}

fn validated_locator(harness: Backend, path: &Path) -> Result<Option<String>, String> {
    match harness {
        Backend::Pi => pi::session_files::validate_session_file(path).map(|_| None),
        Backend::Codex
        | Backend::Cursor
        | Backend::OpenCode
        | Backend::Claude
        | Backend::Antigravity => external_session_locator(harness, path)
            .map(Some)
            .ok_or_else(|| format!("session locator does not belong to {harness}")),
    }
}

pub(crate) fn validate_session_target(target: &SessionTarget) -> Result<(), String> {
    if validated_locator(target.harness, &target.path)?.is_some_and(|id| id != target.id) {
        return Err(format!(
            "session locator does not match its {} identity",
            target.harness
        ));
    }
    Ok(())
}

pub(crate) fn supports_session_move(harness: Backend) -> bool {
    super::known_backend_descriptors()
        .into_iter()
        .any(|backend| {
            backend.id == harness
                && backend.capabilities.sessions.move_project
                    == crate::agents::contract::CapabilitySupport::Available
        })
}

pub(crate) fn validate_session_move(family: &[SessionSummary]) -> Result<(), String> {
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

pub(crate) fn move_session_family(
    family: &[SessionSummary],
    target_project: &Path,
) -> Result<SessionTransfer, String> {
    validate_session_move(family)?;
    let root = &family[0];
    match root.harness {
        Backend::Pi => {
            let members = family
                .iter()
                .map(|session| TransferMember {
                    path: session.path.clone(),
                    id: session.id.clone(),
                    parent_id: session.parent_session.clone(),
                })
                .collect::<Vec<_>>();
            pi::transfer::move_to_project(&members, &root.id, target_project, &root.path)
        }
        Backend::OpenCode => opencode::move_family(family, target_project),
        Backend::Codex => codex::move_family(family, target_project),
        Backend::Cursor | Backend::Claude | Backend::Antigravity => Err(format!(
            "unsupported session move harness: {}",
            root.harness
        )),
    }
}

pub(crate) fn delete_session_family(
    targets: &[SessionTarget],
) -> Result<Vec<(PathBuf, String)>, String> {
    if targets.is_empty() {
        return Err("session family is empty".into());
    }
    for target in targets {
        validate_session_target(target)?;
        if target.harness == Backend::Claude
            || super::external_acp_profile(target.harness).is_some()
        {
            return Err(format!(
                "Session deletion is not supported for {}",
                target.harness
            ));
        }
    }
    let mut pi_paths = Vec::new();
    for target in targets.iter().rev() {
        match target.harness {
            Backend::Pi => pi_paths.push(target.path.clone()),
            Backend::Codex => codex::delete_session(&target.id)?,
            Backend::Cursor => cursor::delete_session(&target.id)?,
            Backend::OpenCode => opencode::delete_session(&target.id)?,
            Backend::Claude | Backend::Antigravity => {
                unreachable!("all session targets were validated")
            }
        }
    }
    if pi_paths.is_empty() {
        Ok(Vec::new())
    } else {
        pi::deletion::delete_family(&pi_paths)
    }
}

pub(crate) fn load_session_history(harness: Backend, path: &Path) -> Result<LoadedHistory, String> {
    validate_session_locator(harness, path)?;
    let history = match harness {
        Backend::Pi => return pi::session_files::load_history(path),
        Backend::Codex => codex::load_history(path)?,
        Backend::Cursor => cursor::load_history(path)?,
        Backend::OpenCode => opencode::load_history(path)?,
        Backend::Antigravity => {
            return Err(
                "Antigravity ACP does not expose history replay through this adapter".into(),
            );
        }
        Backend::Claude => super::claude::load_history(path)?,
    };
    Ok(LoadedHistory {
        messages: history.messages,
        model: history.model,
        thinking_level: history.thinking_level,
        pending_question: None,
    })
}

pub(crate) fn discover_sessions_for(
    harness: Backend,
    locator_root: Option<&Path>,
    query: &str,
) -> Result<Vec<SessionSummary>, String> {
    if harness == Backend::Pi {
        return Ok(pi::session_files::discover(query)?.sessions);
    }
    super::discover_external_sessions_for(harness, locator_root, query)
        .map(|sessions| sessions.into_iter().map(import_session).collect())
}

fn import_session(session: crate::agents::DiscoveredSession) -> SessionSummary {
    let mut summary = SessionSummary::import(crate::sessions::SessionImport {
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
        usage: crate::sessions::UsageSummary {
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
