//! Externally installed ACP agents. No runtime installation or credential storage.
use std::path::Path;

use super::AcpProfile;
use crate::agents::contract::{
    AgentBackendDescriptor, AgentBackendId, AgentCapabilities, CapabilitySupport,
    ConfigurationCapabilities, InteractionCapabilities, ObservationCapabilities,
    SessionCapabilities, TurnCapabilities,
};

pub(in crate::modules::agents::adapter) fn descriptor(
    profile: &AcpProfile,
    replays_history: bool,
) -> AgentBackendDescriptor {
    use CapabilitySupport::{Available, Unsupported};
    let history = if replays_history {
        Available
    } else {
        Unsupported
    };
    AgentBackendDescriptor {
        id: AgentBackendId::new(profile.backend).expect("valid ACP backend id"),
        name: profile.name.into(),
        capabilities: AgentCapabilities {
            sessions: SessionCapabilities {
                list: history.clone(),
                history,
                resume: Available,
                fork: Unsupported,
                rename: Unsupported,
                move_project: Unsupported,
                close: Available,
                delete: Unsupported,
            },
            turns: TurnCapabilities {
                prompt: Available,
                images: Available,
                interrupt: Available,
                steer: Unsupported,
                follow_up: Available,
                compact: Unsupported,
                queue: Available,
            },
            configuration: ConfigurationCapabilities {
                models: Available,
                select_model: Available,
                reasoning_effort: Available,
                modes: Available,
                commands: Available,
                mcp_servers: Available,
            },
            interactions: InteractionCapabilities {
                approvals: Available,
                questions: Unsupported,
                notifications: Available,
            },
            observation: ObservationCapabilities {
                streamed_text: Available,
                reasoning: Available,
                tool_activity: Available,
                usage: Unsupported,
                child_agents: Unsupported,
                file_changes: Available,
            },
        },
    }
}

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;

pub(in crate::modules::agents::adapter) fn discover(
    profile: &AcpProfile,
    root: &Path,
    query: &str,
) -> Result<Vec<crate::agents::DiscoveredSession>, String> {
    let query = query.to_lowercase();
    Ok(super::list_sessions(profile)?
        .iter()
        .filter_map(|entry| {
            let id = entry.get("sessionId")?.as_str()?;
            if id.is_empty() || id.contains(['/', '\\']) || id == "." || id == ".." {
                return None;
            }
            let project = std::path::PathBuf::from(entry.get("cwd")?.as_str()?);
            if !project.is_absolute()
                || !project.is_dir()
                || crate::projects::is_temporary_project(&project)
            {
                return None;
            }
            let title = entry
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(profile.name)
                .to_owned();
            let search = format!("{title} {} {}", project.display(), profile.name);
            if !search.to_lowercase().contains(&query) {
                return None;
            }
            let timestamp = entry
                .get("updatedAt")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let modified = time::OffsetDateTime::parse(
                &timestamp,
                &time::format_description::well_known::Rfc3339,
            )
            .ok()
            .map(std::time::SystemTime::from)
            .unwrap_or(std::time::UNIX_EPOCH);
            Some(crate::agents::DiscoveredSession {
                id: id.into(),
                harness: profile.backend.into(),
                path: super::super::main_session::external_session_path(root, profile.backend, id),
                project,
                title,
                timestamp,
                modified,
                search,
                first_user_message: String::new(),
                parent_session: None,
                message_count: 0,
                usage: Default::default(),
                archived: false,
                is_running: false,
                model: None,
                thinking_level: None,
            })
        })
        .collect())
}

pub(in crate::modules::agents::adapter) fn load_history(
    profile: &AcpProfile,
    path: &Path,
) -> Result<crate::agents::DiscoveredHistory, String> {
    let id = super::super::main_session::external_session_locator(profile.backend, path)
        .ok_or("Invalid ACP session locator")?;
    let entry = super::list_sessions(profile)?
        .into_iter()
        .find(|entry| entry.get("sessionId").and_then(serde_json::Value::as_str) == Some(&id))
        .ok_or("ACP session was not found")?;
    let project = entry
        .get("cwd")
        .and_then(serde_json::Value::as_str)
        .ok_or("ACP session omitted cwd")?;
    super::load_history(profile, path, Path::new(project))
}
