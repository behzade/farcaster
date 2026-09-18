use super::acp::AcpProfile;
use crate::agents::Backend;

pub(super) const PROFILE: AcpProfile = AcpProfile {
    backend: Backend::Antigravity,
    name: "Antigravity",
    command: if cfg!(windows) {
        "agy_acp_server.exe"
    } else {
        "agy_acp_server.par"
    },
    path_environment: "FARCASTER_ANTIGRAVITY_ACP_PATH",
    arguments: if cfg!(target_os = "linux") {
        &["--uid="]
    } else {
        &[]
    },
    // The official server is already signed in. Sending authenticate for
    // oauth-personal is unnecessary and can stall session startup.
    auth_method: None,
    force_argument: None,
    resume_method: "session/resume",
    permission_modes: Some(("default", "yolo")),
};

pub(super) fn descriptor() -> crate::agents::contract::AgentBackendDescriptor {
    super::acp::backend::descriptor(&PROFILE, false)
}

/// The official server persists each conversation's working directory beside
/// its trajectory database under the Gemini home.
fn conversation_project(session_id: &str) -> Option<std::path::PathBuf> {
    let home = std::env::var_os("GEMINI_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".gemini"))
        })?;
    conversation_project_in(&home, session_id)
}

fn conversation_project_in(home: &std::path::Path, session_id: &str) -> Option<std::path::PathBuf> {
    let meta = home
        .join("antigravity-acp")
        .join("conversations")
        .join(format!("{session_id}.meta"));
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&meta).ok()?).ok()?;
    Some(std::path::PathBuf::from(value.get("cwd")?.as_str()?))
}

pub(super) fn load_history(
    path: &std::path::Path,
    fallback_project: &std::path::Path,
) -> Result<crate::agents::DiscoveredHistory, String> {
    let id = super::main_session::external_session_locator(PROFILE.backend, path)
        .ok_or_else(|| format!("invalid Antigravity session locator: {}", path.display()))?;
    let project = conversation_project(&id).unwrap_or_else(|| fallback_project.to_owned());
    super::acp::load_history(&PROFILE, path, &project)
}

pub(super) fn configure(command: &mut std::process::Command) -> Result<(), String> {
    let helper = std::path::Path::new(command.get_program())
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(if cfg!(windows) {
            "localharness_external.exe"
        } else {
            "localharness_external"
        });
    if !helper.is_file() {
        return Err(format!(
            "Antigravity ACP requires its matching helper beside the executable: {}",
            helper.display()
        ));
    }
    command.env("ANTIGRAVITY_HARNESS_PATH", helper);
    Ok(())
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
