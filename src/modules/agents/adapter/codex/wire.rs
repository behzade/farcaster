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
#[path = "wire_tests.rs"]
mod tests;
