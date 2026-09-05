mod catalog;

use std::path::Path;

use super::super::contract::{
    AgentBackendDescriptor, AgentBackendId, AgentCapabilities, CapabilitySupport,
    ConfigurationCapabilities, InteractionCapabilities, ObservationCapabilities,
    SessionCapabilities, TurnCapabilities,
};
use super::acp::{AcpProfile, AcpWorkerFactory};

pub(super) const PROFILE: AcpProfile = AcpProfile {
    backend: "cursor-cli",
    name: "Cursor",
    command: "agent",
    path_environment: "FARCASTER_CURSOR_PATH",
    arguments: &["acp"],
    auth_method: Some("cursor_login"),
    force_argument: Some("--force"),
};

pub(crate) fn descriptor() -> AgentBackendDescriptor {
    use CapabilitySupport::{Available, Unsupported};

    AgentBackendDescriptor {
        id: AgentBackendId::new(PROFILE.backend).expect("Cursor backend id is valid"),
        name: PROFILE.name.into(),
        capabilities: AgentCapabilities {
            sessions: SessionCapabilities {
                list: Available,
                history: Available,
                resume: Available,
                fork: Unsupported,
                rename: Available,
                close: Available,
                delete: Available,
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
                reasoning_effort: Unsupported,
                modes: Available,
                commands: Available,
                mcp_servers: Available,
            },
            interactions: InteractionCapabilities {
                approvals: Available,
                questions: Available,
                notifications: Available,
            },
            observation: ObservationCapabilities {
                streamed_text: Available,
                reasoning: Available,
                tool_activity: Available,
                usage: Unsupported,
                child_agents: Available,
                file_changes: Available,
            },
        },
    }
}

pub(super) fn worker_factory(mut command: crate::agents::AgentLaunchConfig) -> AcpWorkerFactory {
    command.program = PROFILE.program();
    AcpWorkerFactory::new(command, PROFILE)
}

pub(super) fn spawn_main(
    command: &crate::agents::AgentLaunchConfig,
    launch: &crate::agents::SessionLaunch,
) -> Result<
    (
        Box<dyn crate::agents::WorkerSession>,
        String,
        super::main_session::MainSessionMetadata,
    ),
    String,
> {
    if let crate::agents::SessionStart::Resume(_) = &launch.start {
        let id = super::main_session::launch_session_locator(launch)
            .ok_or_else(|| "Cursor resume requires a session id".to_owned())?;
        if catalog::inspect(&id)?.1 {
            let fresh = crate::agents::SessionLaunch {
                harness: launch.harness.clone(),
                session_id: None,
                project: launch.project.clone(),
                start: crate::agents::SessionStart::New,
                wake: launch.wake.clone(),
            };
            return super::acp::spawn_main(command, &PROFILE, &fresh);
        }
    }
    super::acp::spawn_main(command, &PROFILE, launch)
}

pub(super) use catalog::{delete as delete_session, rename as rename_session};

pub(super) fn load_configuration(
    project: &Path,
) -> Result<super::main_session::MainSessionMetadata, String> {
    let (metadata, session_id) = super::acp::load_configuration(&PROFILE, project)?;
    let _ = catalog::delete(&session_id);
    Ok(metadata)
}

pub(super) fn discover(
    locator_root: &Path,
    query: &str,
) -> Result<Vec<crate::agents::DiscoveredSession>, String> {
    catalog::discover(locator_root, query)
}

pub(super) fn load_history(path: &Path) -> Result<crate::agents::DiscoveredHistory, String> {
    history(path, None)
}

pub(super) fn load_history_at(
    path: &Path,
    project: &Path,
) -> Result<crate::agents::DiscoveredHistory, String> {
    history(path, Some(project))
}

fn history(
    path: &Path,
    project: Option<&Path>,
) -> Result<crate::agents::DiscoveredHistory, String> {
    let id = super::main_session::external_session_locator(PROFILE.backend, path)
        .ok_or_else(|| format!("invalid Cursor session locator: {}", path.display()))?;
    let (stored_project, unpersisted) = catalog::inspect(&id)?;
    if unpersisted {
        return Ok(crate::agents::DiscoveredHistory {
            messages: Vec::new(),
            model: None,
            thinking_level: None,
        });
    }
    super::acp::load_history(&PROFILE, path, project.unwrap_or(&stored_project))
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "queries the configured live Cursor account"]
    fn live_cursor_configuration_catalog() -> Result<(), String> {
        let project = std::env::current_dir().map_err(|error| error.to_string())?;
        let catalog = super::super::load_configuration_catalog(
            &crate::agents::AgentLaunchConfig::default(),
            super::PROFILE.backend,
            &project,
        )?;
        assert!(!catalog.models.is_empty(), "Cursor returned no models");
        assert!(catalog.models.iter().all(|model| {
            model.provider == super::PROFILE.backend && !model.id.is_empty()
        }));
        eprintln!("Cursor catalog loaded {} models", catalog.models.len());
        Ok(())
    }
}
