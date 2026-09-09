use super::acp::AcpProfile;

pub(super) const PROFILE: AcpProfile = AcpProfile {
    backend: "claude-acp",
    name: "Claude Code",
    command: "claude-agent-acp",
    path_environment: "FARCASTER_CLAUDE_ACP_PATH",
    arguments: &[],
    auth_method: None,
    force_argument: None,
    resume_method: "session/load",
    permission_modes: Some(("default", "bypassPermissions")),
};

pub(super) fn descriptor() -> crate::agents::contract::AgentBackendDescriptor {
    super::acp::backend::descriptor(&PROFILE, true)
}
