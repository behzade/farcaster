use super::*;

#[test]
fn preserves_nested_error_details() {
    let message = decode_frame(br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Invalid params","data":{"message":"Session missing not found"}}}"#).unwrap();
    assert!(
        matches!(message, AcpInbound::Error { code: -32602, message, .. }
            if message.contains("Invalid params") && message.contains("Session missing not found"))
    );
    assert_eq!(
        error_message(&serde_json::json!({"message": "Invalid params", "data": null})),
        "Invalid params"
    );
    assert_eq!(
        error_message(&serde_json::json!({"message": "Invalid params"})),
        "Invalid params"
    );
}

#[test]
fn distinguishes_agent_requests_and_notifications() -> Result<(), String> {
    assert!(matches!(
        decode_frame(
            br#"{"jsonrpc":"2.0","id":"p","method":"session/request_permission","params":{}}"#
        )?,
        AcpInbound::AgentRequest { .. }
    ));
    assert!(matches!(
        decode_frame(br#"{"jsonrpc":"2.0","method":"session/update","params":{}}"#)?,
        AcpInbound::Notification { .. }
    ));
    Ok(())
}
