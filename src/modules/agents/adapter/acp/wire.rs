use serde_json::{Map, Value};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) enum AcpRequestId {
    Number(i64),
    String(String),
    Null,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum AcpInbound {
    Response {
        id: AcpRequestId,
        result: Value,
    },
    Error {
        id: AcpRequestId,
        code: i64,
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

pub(super) fn encode_request(
    id: &AcpRequestId,
    method: &str,
    params: Value,
) -> Result<Vec<u8>, String> {
    encode(Map::from_iter([
        ("jsonrpc".into(), Value::String("2.0".into())),
        ("id".into(), id_value(id)),
        ("method".into(), Value::String(method.into())),
        ("params".into(), params),
    ]))
}

pub(super) fn encode_notification(method: &str, params: Value) -> Result<Vec<u8>, String> {
    encode(Map::from_iter([
        ("jsonrpc".into(), Value::String("2.0".into())),
        ("method".into(), Value::String(method.into())),
        ("params".into(), params),
    ]))
}

pub(super) fn encode_response(id: &AcpRequestId, result: Value) -> Result<Vec<u8>, String> {
    encode(Map::from_iter([
        ("jsonrpc".into(), Value::String("2.0".into())),
        ("id".into(), id_value(id)),
        ("result".into(), result),
    ]))
}

fn encode(value: Map<String, Value>) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(&Value::Object(value))
        .map_err(|error| format!("encode ACP message: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(super) fn decode_frame(frame: &[u8]) -> Result<AcpInbound, String> {
    let value: Value =
        serde_json::from_slice(frame).map_err(|error| format!("malformed ACP frame: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "ACP frame is not an object".to_owned())?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err("ACP frame does not declare JSON-RPC 2.0".into());
    }
    let id = object.get("id").map(request_id).transpose()?;
    let method = object.get("method").and_then(Value::as_str);
    match (id, method, object.get("result"), object.get("error")) {
        (Some(id), Some(method), None, None) => Ok(AcpInbound::AgentRequest {
            id,
            method: method.into(),
            params: object.get("params").cloned().unwrap_or(Value::Null),
        }),
        (None, Some(method), None, None) => Ok(AcpInbound::Notification {
            method: method.into(),
            params: object.get("params").cloned().unwrap_or(Value::Null),
        }),
        (Some(id), None, Some(result), None) => Ok(AcpInbound::Response {
            id,
            result: result.clone(),
        }),
        (Some(id), None, None, Some(error)) => Ok(AcpInbound::Error {
            id,
            code: error.get("code").and_then(Value::as_i64).unwrap_or(-1),
            message: error_message(error),
        }),
        _ => Err("unrecognized ACP frame shape".into()),
    }
}

// ACP agents often put the actionable explanation in JSON-RPC error.data.
fn error_message(error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown ACP error");
    match error.get("data").filter(|data| !data.is_null()) {
        Some(data) => format!("{message}: {data}"),
        None => message.to_owned(),
    }
}

fn request_id(value: &Value) -> Result<AcpRequestId, String> {
    if value.is_null() {
        return Ok(AcpRequestId::Null);
    }
    value
        .as_i64()
        .map(AcpRequestId::Number)
        .or_else(|| {
            value
                .as_str()
                .map(|value| AcpRequestId::String(value.into()))
        })
        .ok_or_else(|| "ACP request id must be a string, integer, or null".into())
}

fn id_value(id: &AcpRequestId) -> Value {
    match id {
        AcpRequestId::Number(value) => Value::Number((*value).into()),
        AcpRequestId::String(value) => Value::String(value.clone()),
        AcpRequestId::Null => Value::Null,
    }
}

#[cfg(test)]
mod tests {
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
}
