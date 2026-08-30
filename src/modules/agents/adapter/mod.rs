#[allow(dead_code)]
mod codex;
#[allow(dead_code)]
mod opencode;
mod pi;

pub(super) use pi::encode_request as encode_pi_request;
pub(crate) use pi::{
    PiEvent, PiProcessCommand, PiRequest, PiResponse, PiRpcProcess, PiSessionTransport,
    PiWorkerFactory, descriptor as pi_descriptor,
};
#[cfg(test)]
pub(crate) use pi::{PiWireMessage, parse_frame};

pub(super) fn known_backend_descriptors() -> [crate::agents::AgentBackendDescriptor; 3] {
    [
        pi::descriptor(),
        codex::descriptor(),
        opencode::descriptor(),
    ]
}
