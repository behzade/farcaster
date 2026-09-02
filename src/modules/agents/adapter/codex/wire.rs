use serde_json::{Map, Value};

use super::contract::{CodexInbound, CodexRequestId, CodexRpcError};

pub(super) fn encode_request(
    id: &CodexRequestId,
    method: &str,
    params: Value,
) -> Result<Vec<u8>, String> {
    encode_message(Map::from_iter([
        ("id".into(), serde_json::to_value(id).map_err(json_error)?),
        ("method".into(), Value::String(method.into())),
        ("params".into(), params),
    ]))
}

pub(super) fn encode_notification(method: &str, params: Option<Value>) -> Result<Vec<u8>, String> {
    let mut value = Map::from_iter([("method".into(), Value::String(method.into()))]);
    if let Some(params) = params {
        value.insert("params".into(), params);
    }
    encode_message(value)
}

pub(super) fn encode_response(id: &CodexRequestId, result: Value) -> Result<Vec<u8>, String> {
    encode_message(Map::from_iter([
        ("id".into(), serde_json::to_value(id).map_err(json_error)?),
        ("result".into(), result),
    ]))
}

pub(super) fn encode_error_response(
    id: &CodexRequestId,
    code: i64,
    message: &str,
) -> Result<Vec<u8>, String> {
    encode_message(Map::from_iter([
        ("id".into(), serde_json::to_value(id).map_err(json_error)?),
        (
            "error".into(),
            serde_json::json!({"code": code, "message": message}),
        ),
    ]))
}

fn encode_message(value: Map<String, Value>) -> Result<Vec<u8>, String> {
    let mut encoded = serde_json::to_vec(&Value::Object(value)).map_err(json_error)?;
    encoded.push(b'\n');
    Ok(encoded)
}

pub(super) fn decode_frame(frame: &[u8]) -> Result<CodexInbound, String> {
    let value: Value = serde_json::from_slice(frame)
        .map_err(|error| format!("malformed Codex app-server frame: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "Codex app-server frame is not an object".to_owned())?;
    let id = object
        .get("id")
        .map(|value| serde_json::from_value::<CodexRequestId>(value.clone()).map_err(json_error))
        .transpose()?;
    let method = object.get("method").and_then(Value::as_str);
    match (id, method, object.get("result"), object.get("error")) {
        (Some(id), Some(method), None, None) => Ok(CodexInbound::ServerRequest {
            id,
            method: method.into(),
            params: object.get("params").cloned().unwrap_or(Value::Null),
        }),
        (None, Some(method), None, None) => Ok(CodexInbound::Notification {
            method: method.into(),
            params: object.get("params").cloned().unwrap_or(Value::Null),
        }),
        (Some(id), None, Some(result), None) => Ok(CodexInbound::Response {
            id,
            result: result.clone(),
        }),
        (Some(id), None, None, Some(error)) => {
            serde_json::from_value::<CodexRpcError>(error.clone())
                .map(|error| CodexInbound::Error { id, error })
                .map_err(json_error)
        }
        _ => Err("unrecognized Codex app-server frame shape".into()),
    }
}

fn json_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
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
            decode_frame(br#"{"id":"approval-1","method":"item/commandExecution/requestApproval","params":{}}"#)?,
            CodexInbound::ServerRequest {
                id: CodexRequestId::String("approval-1".into()),
                method: "item/commandExecution/requestApproval".into(),
                params: json!({}),
            }
        );
        Ok(())
    }
}
