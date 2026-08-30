mod adapter;
mod contract;

pub(crate) use adapter::{
    PiEvent, PiProcessCommand, PiRequest, PiResponse, PiRpcProcess, PiSessionTransport,
    PiWorkerFactory, pi_descriptor,
};
#[cfg(test)]
pub(crate) use adapter::{PiWireMessage, parse_frame};
pub(crate) use contract::{
    AgentBackendDescriptor, AgentBackendId, AgentCapabilities, CapabilitySupport,
    ConfigurationCapabilities, InteractionCapabilities, ObservationCapabilities,
    SessionCapabilities, TurnCapabilities,
};

pub(crate) fn encode_pi_request(request: PiRequest) -> serde_json::Value {
    adapter::encode_pi_request(request)
}

#[allow(dead_code)] // Native Codex and OpenCode runtimes are not composed yet.
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
