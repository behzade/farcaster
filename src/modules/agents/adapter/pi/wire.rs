use serde::Deserialize;
use serde_json::Value;

use crate::agents::extensions::ExtensionUiRequest;

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct PiResponse {
    pub id: Option<String>,
    pub command: String,
    pub success: bool,
    #[serde(default)]
    pub data: Value,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PiWireMessage {
    Response(PiResponse),
    ExtensionUi(ExtensionUiRequest),
    Event(Value),
}

#[derive(Deserialize)]
struct ExtensionEnvelope {
    method: String,
    id: Option<String>,
}

pub(crate) fn parse_frame(frame: &[u8]) -> Result<PiWireMessage, String> {
    let value: Value =
        serde_json::from_slice(frame).map_err(|error| format!("malformed JSON frame: {error}"))?;
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return Err("JSON frame has no string type".to_owned());
    };
    match kind {
        "response" => serde_json::from_value(value)
            .map(PiWireMessage::Response)
            .map_err(|error| format!("invalid response frame: {error}")),
        "extension_ui_request" => parse_extension_request(value).map(PiWireMessage::ExtensionUi),
        _ => Ok(PiWireMessage::Event(value)),
    }
}

fn parse_extension_request(value: Value) -> Result<ExtensionUiRequest, String> {
    let envelope = ExtensionEnvelope::deserialize(&value)
        .map_err(|error| format!("invalid extension UI request: {error}"))?;
    if !matches!(
        envelope.method.as_str(),
        "select"
            | "confirm"
            | "input"
            | "editor"
            | "notify"
            | "setStatus"
            | "setWidget"
            | "setTitle"
            | "set_editor_text"
    ) {
        return Ok(ExtensionUiRequest::Unknown {
            id: envelope.id.filter(|id| !id.is_empty()),
            method: envelope.method,
        });
    }
    serde_json::from_value(value)
        .map_err(|error| format!("invalid {} extension UI request: {error}", envelope.method))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_response_and_activity_frames() {
        assert_eq!(
            parse_frame(br#"{"type":"response","id":"1","command":"abort","success":true}"#),
            Ok(PiWireMessage::Response(PiResponse {
                id: Some("1".into()),
                command: "abort".into(),
                success: true,
                data: Value::Null,
                error: None,
            }))
        );
        assert_eq!(
            parse_frame(br#"{"type":"agent_start"}"#),
            Ok(PiWireMessage::Event(
                serde_json::json!({"type":"agent_start"})
            ))
        );
    }

    #[test]
    fn keeps_unknown_extension_methods_observable() {
        assert_eq!(
            parse_frame(br#"{"type":"extension_ui_request","id":"u1","method":"futureMethod"}"#),
            Ok(PiWireMessage::ExtensionUi(ExtensionUiRequest::Unknown {
                id: Some("u1".into()),
                method: "futureMethod".into(),
            }))
        );
    }

    #[test]
    fn rejects_malformed_frames() {
        assert!(parse_frame(b"{").is_err());
        assert!(parse_frame(br#"{"command":"abort"}"#).is_err());
    }
}
