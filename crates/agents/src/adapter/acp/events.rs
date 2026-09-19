use serde_json::Value;

pub(super) use agent_client_protocol::schema::v1::RequestId as AcpRequestId;

/// Events delivered by the SDK to Farcaster's adapter.
#[derive(Clone, Debug)]
pub(super) enum AcpInbound {
    Response {
        id: AcpRequestId,
        result: Value,
    },
    Error {
        id: AcpRequestId,
        message: String,
    },
    Notification {
        method: String,
        params: Value,
    },
    AgentRequest {
        id: AcpRequestId,
        method: String,
        params: Value,
    },
}
