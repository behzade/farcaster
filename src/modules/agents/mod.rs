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
