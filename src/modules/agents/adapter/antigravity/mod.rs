use super::acp::AcpProfile;

pub(super) const PROFILE: AcpProfile = AcpProfile {
    backend: "antigravity-acp",
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
    // The official server selects its saved Google account through ACP authenticate.
    auth_method: Some("oauth-personal"),
    force_argument: None,
    resume_method: "session/resume",
    permission_modes: Some(("default", "yolo")),
};

pub(super) fn descriptor() -> crate::agents::contract::AgentBackendDescriptor {
    super::acp::backend::descriptor(&PROFILE, false)
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
