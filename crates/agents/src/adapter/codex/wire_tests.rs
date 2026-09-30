use super::*;
use serde_json::json;

#[test]
fn app_server_wire_omits_jsonrpc_marker() -> Result<(), String> {
    let encoded = encode_request(
        &CodexRequestId::Number(7),
        "turn/interrupt",
        json!({"threadId":"thread-1","turnId":"turn-1"}),
    )?;
    assert_eq!(
        String::from_utf8(encoded).map_err(|error| error.to_string())?,
        "{\"id\":7,\"method\":\"turn/interrupt\",\"params\":{\"threadId\":\"thread-1\",\"turnId\":\"turn-1\"}}\n"
    );
    let error = encode_error_response(&CodexRequestId::Number(7), -32601, "unsupported")?;
    assert_eq!(
        String::from_utf8(error).map_err(|error| error.to_string())?,
        "{\"id\":7,\"error\":{\"code\":-32601,\"message\":\"unsupported\"}}\n"
    );
    Ok(())
}

#[test]
fn distinguishes_notifications_from_server_requests() -> Result<(), String> {
    assert_eq!(
        decode_frame(br#"{"method":"item/agentMessage/delta","params":{"delta":"hi"}}"#)?,
        CodexInbound::Notification {
            method: "item/agentMessage/delta".into(),
            params: json!({"delta":"hi"}),
        }
    );
    assert_eq!(
        decode_frame(
            br#"{"id":"approval-1","method":"item/commandExecution/requestApproval","params":{}}"#
        )?,
        CodexInbound::ServerRequest {
            id: CodexRequestId::String("approval-1".into()),
            method: "item/commandExecution/requestApproval".into(),
            params: json!({}),
        }
    );
    Ok(())
}

#[test]
fn decodes_null_results_and_error_data_without_changing_frame_validation() {
    assert_eq!(
        decode_frame(br#"{"id":1,"result":null,"method":false}"#).unwrap(),
        CodexInbound::Response {
            id: CodexRequestId::Number(1),
            result: Value::Null
        }
    );
    assert_eq!(
        decode_frame(
            br#"{"id":"request","error":{"code":42,"message":"failed","data":{"detail":[1,2]}}}"#
        )
        .unwrap(),
        CodexInbound::Error {
            id: CodexRequestId::String("request".into()),
            error: CodexRpcError {
                code: 42,
                message: "failed".into(),
                data: json!({"detail":[1,2]})
            },
        }
    );
    assert_eq!(
        decode_frame(br#"{"method":"notice"}"#).unwrap(),
        CodexInbound::Notification {
            method: "notice".into(),
            params: Value::Null
        }
    );
    for frame in [
        "[]",
        "{}",
        "{\"id\":null,\"method\":\"notice\"}",
        "{\"id\":1,\"result\":null,\"error\":null}",
        "{\"id\":1,\"method\":\"request\",\"result\":null}",
        "{\"method\":\"notice\",\"error\":{}}",
        "{\"id\":1,\"error\":{\"message\":\"missing code\"}}",
    ] {
        assert!(decode_frame(frame.as_bytes()).is_err(), "{frame}");
    }
}
