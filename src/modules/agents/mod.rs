mod adapter;
mod contract;
mod core;

pub(crate) use adapter::{
    AgentProcessCommand, CodexWorkerFactory, OpenCodeWorkerFactory, PiEvent, PiProcessCommand,
    PiRequest, PiResponse, PiRpcProcess, PiSessionTransport, PiWorkerFactory,
    app_shell_environment, default_login_shell,
};
#[cfg(test)]
pub(crate) use adapter::{PiWireMessage, parse_frame};
pub(crate) use contract::extensions;
pub(crate) use contract::{
    AgentBackendDescriptor, AgentBackendId, AgentCapabilities, CapabilitySupport,
    ConfigurationCapabilities, FileAccessMode, InteractionCapabilities, NetworkAccessMode,
    ObservationCapabilities, PermissionLevel, SessionCapabilities, TurnCapabilities, WorkerContext,
    WorkerInput, WorkerInputResponse,
};
pub(crate) use core::{
    CallerIdentity, CallerRegistry, WorkerEvent, WorkerLaunch, WorkerSendMode, WorkerSession,
    WorkerSessionFactory,
};

pub(crate) fn encode_pi_request(request: PiRequest) -> serde_json::Value {
    adapter::encode_pi_request(request)
}

pub(crate) fn known_backend_descriptors() -> [AgentBackendDescriptor; 3] {
    adapter::known_backend_descriptors()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_catalog_preserves_backend_specific_capabilities() {
        let [pi, codex, opencode] = known_backend_descriptors();
        assert_eq!(pi.id.as_str(), "pi");
        assert_eq!(codex.id.as_str(), "codex-cli");
        assert_eq!(opencode.id.as_str(), "opencode2");
        assert_eq!(
            pi.capabilities.turns.follow_up,
            CapabilitySupport::Available
        );
        assert_eq!(
            codex.capabilities.turns.follow_up,
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            opencode.capabilities.configuration.commands,
            CapabilitySupport::Available
        );
    }
}
