mod catalog;

use std::{path::Path, time::Instant};

use super::super::contract::{
    AgentBackendDescriptor, AgentCapabilities, Backend, CapabilitySupport,
    ConfigurationCapabilities, InteractionCapabilities, ObservationCapabilities,
    SessionCapabilities, TurnCapabilities,
};
use super::acp::{AcpProfile, AcpWorkerFactory};

pub(super) const PROFILE: AcpProfile = AcpProfile {
    backend: Backend::Cursor,
    name: "Cursor",
    command: "agent",
    path_environment: "FARCASTER_CURSOR_PATH",
    arguments: &["acp"],
    // Cursor ACP uses the already-signed-in CLI. Sending authenticate for
    // cursor_login breaks session startup.
    auth_method: None,
    force_argument: Some("--force"),
    resume_method: "session/load",
    permission_modes: None,
};

pub(crate) fn descriptor() -> AgentBackendDescriptor {
    use crate::agents::HarnessAccessMode::{Auto, Full, Sandboxed};
    use CapabilitySupport::{Available, Unsupported};

    AgentBackendDescriptor {
        id: PROFILE.backend,
        name: PROFILE.name.into(),
        capabilities: AgentCapabilities {
            sessions: SessionCapabilities {
                list: Available,
                history: Available,
                resume: Available,
                fork: Unsupported,
                rename: Available,
                move_project: Unsupported,
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
                access_modes: &[Sandboxed, Auto, Full],
                model_required_access_modes: &[],
                models: Available,
                select_model: Available,
                reasoning_effort: Available,
                effort_label: "Effort",
                reset_reasoning_effort: CapabilitySupport::Unsupported,
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
) -> Result<super::acp::MainSession, String> {
    if let crate::agents::SessionStart::Resume(_) = &launch.start {
        let id = super::main_session::launch_session_locator(launch)
            .ok_or_else(|| "Cursor resume requires a session id".to_owned())?;
        if catalog::inspect(&id)?.1 {
            let fresh = crate::agents::SessionLaunch {
                harness: launch.harness,
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
    let started = Instant::now();
    let id = super::main_session::external_session_locator(PROFILE.backend, path)
        .ok_or_else(|| format!("invalid Cursor session locator: {}", path.display()))?;
    let (stored_project, unpersisted) = catalog::inspect(&id)?;
    if unpersisted {
        return Ok(crate::agents::DiscoveredHistory {
            messages: Vec::new(),
            model: None,
            thinking_level: None,
            prompt_deliveries: None,
        });
    }
    let history = super::acp::load_history(&PROFILE, path, &stored_project);
    zlog::info!(
        "PERF operation=history.cursor.load elapsed_ms={:.2}",
        started.elapsed().as_secs_f64() * 1_000.0
    );
    history
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
