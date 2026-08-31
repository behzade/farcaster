#[allow(dead_code)]
mod codex;
mod farcaster_mcp;
#[allow(dead_code)]
mod opencode;
mod pi;
mod process_command;
mod shell_environment;

pub(crate) use codex::CodexWorkerFactory;
pub(crate) use opencode::OpenCodeWorkerFactory;
pub(super) use pi::encode_request as encode_pi_request;
pub(crate) use pi::{
    PiEvent, PiProcessCommand, PiRequest, PiResponse, PiRpcProcess, PiSessionTransport,
    PiWorkerFactory,
};
#[cfg(test)]
pub(crate) use pi::{PiWireMessage, parse_frame};
pub(crate) use process_command::AgentProcessCommand;
pub(crate) use shell_environment::{app_shell_environment, default_login_shell};

pub(super) fn known_backend_descriptors() -> [crate::agents::AgentBackendDescriptor; 3] {
    [
        pi::descriptor(),
        codex::descriptor(),
        opencode::descriptor(),
    ]
}
