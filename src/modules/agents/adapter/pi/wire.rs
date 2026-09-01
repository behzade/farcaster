use serde::Deserialize;
use serde_json::Value;

use crate::agents::{
    SessionOperation, SessionResponse,
    extensions::{ExtensionUiRequest, PromptMode},
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PiWireMessage {
    Response {
        response: SessionResponse,
        command: String,
    },
    ExtensionUi(ExtensionUiRequest),
    Event(Value),
}

#[derive(Deserialize)]
struct ResponseEnvelope {
    id: Option<String>,
    command: String,
    success: bool,
    #[serde(default)]
    data: Value,
    error: Option<String>,
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
        "response" => {
            let response = ResponseEnvelope::deserialize(value)
                .map_err(|error| format!("invalid response frame: {error}"))?;
            let operation = response_operation(&response.command);
            Ok(PiWireMessage::Response {
                command: response.command,
                response: SessionResponse {
                    id: response.id,
                    operation,
                    success: response.success,
                    data: response.data,
                    error: response.error,
                },
            })
        }
        "extension_ui_request" => parse_extension_request(value).map(PiWireMessage::ExtensionUi),
        _ => Ok(PiWireMessage::Event(value)),
    }
}

fn response_operation(command: &str) -> SessionOperation {
    match command {
        "set_steering_mode" => SessionOperation::ConfigureSteering,
        "get_state" => SessionOperation::LoadState,
        "get_entries" => SessionOperation::LoadHistory,
        "get_session_stats" => SessionOperation::LoadUsage,
        "get_available_models" => SessionOperation::ListModels,
        "get_available_thinking_levels" => SessionOperation::ListReasoningLevels,
        "get_modes" => SessionOperation::ListModes,
        "get_commands" => SessionOperation::ListCommands,
        "prompt" => SessionOperation::Prompt(PromptMode::Normal),
        "steer" => SessionOperation::Prompt(PromptMode::Steer),
        "follow_up" => SessionOperation::Prompt(PromptMode::FollowUp),
        "abort" => SessionOperation::Abort,
        "compact" => SessionOperation::Compact,
        "export_html" => SessionOperation::ExportHtml,
        "set_session_name" => SessionOperation::Rename,
        "fork" => SessionOperation::ForkAt,
        "set_model" => SessionOperation::SelectModel,
        "set_thinking_level" => SessionOperation::SelectReasoning,
        "set_mode" => SessionOperation::SelectMode,
        _ => SessionOperation::Other,
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
            Ok(PiWireMessage::Response {
                command: "abort".into(),
                response: SessionResponse {
                    id: Some("1".into()),
                    operation: SessionOperation::Abort,
                    success: true,
                    data: Value::Null,
                    error: None,
                },
            })
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
