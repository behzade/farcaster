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
            let mut response = ResponseEnvelope::deserialize(value)
                .map_err(|error| format!("invalid response frame: {error}"))?;
            if response.success && response.command == "get_available_models" {
                add_model_efforts(&mut response.data);
            }
            if response.success && response.command == "get_session_stats" {
                add_usage_total(response.data.get_mut("tokens"));
            }
            if response.success && response.command == "get_entries" {
                super::tool::annotate_pi_value(&mut response.data);
            }
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
        _ => {
            let is_turn_end = kind == "turn_end";
            let mut value = value;
            if is_turn_end {
                add_usage_total(value.get_mut("usage"));
            }
            super::tool::annotate_pi_value(&mut value);
            Ok(PiWireMessage::Event(value))
        }
    }
}

const PI_THINKING_LEVELS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

fn add_usage_total(usage: Option<&mut Value>) {
    let Some(usage) = usage.and_then(Value::as_object_mut) else {
        return;
    };
    if usage.get("totalTokens").and_then(Value::as_u64).is_some() {
        return;
    }
    let total = ["input", "output", "cacheRead", "cacheWrite"]
        .into_iter()
        .filter_map(|field| usage.get(field).and_then(Value::as_u64))
        .fold(0_u64, u64::saturating_add);
    usage.insert("totalTokens".into(), Value::from(total));
}

fn add_model_efforts(data: &mut Value) {
    let models = data
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten();
    for model in models.filter_map(Value::as_object_mut) {
        if model.contains_key("efforts") {
            continue;
        }
        let efforts = if model.get("reasoning").and_then(Value::as_bool) == Some(true) {
            let mappings = model.get("thinkingLevelMap").and_then(Value::as_object);
            PI_THINKING_LEVELS
                .into_iter()
                .filter(
                    |level| match mappings.and_then(|mappings| mappings.get(*level)) {
                        Some(Value::Null) => false,
                        Some(_) => true,
                        None => !matches!(*level, "xhigh" | "max"),
                    },
                )
                .collect()
        } else {
            vec!["off"]
        };
        model.insert("efforts".into(), serde_json::json!(efforts));
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
    fn normalizes_missing_usage_totals() {
        let PiWireMessage::Event(event) = parse_frame(
            br#"{"type":"turn_end","usage":{"input":10,"output":2,"cacheRead":3,"cacheWrite":1}}"#,
        )
        .expect("turn event") else {
            panic!("expected event");
        };
        assert_eq!(event["usage"]["totalTokens"], 16);

        let PiWireMessage::Response { response, .. } = parse_frame(
            br#"{"type":"response","command":"get_session_stats","success":true,"data":{"tokens":{"input":5,"output":1,"cacheRead":2,"cacheWrite":0}}}"#,
        )
        .expect("usage response")
        else {
            panic!("expected response");
        };
        assert_eq!(response.data["tokens"]["totalTokens"], 8);
    }

    #[test]
    fn adds_per_model_efforts_to_pi_model_catalogs() {
        let frame = serde_json::to_vec(&serde_json::json!({
            "type": "response",
            "command": "get_available_models",
            "success": true,
            "data": {"models": [
                {"id": "plain", "reasoning": false},
                {"id": "default", "reasoning": true},
                {"id": "mapped", "reasoning": true, "thinkingLevelMap": {
                    "minimal": null,
                    "xhigh": "xhigh",
                    "max": null
                }},
                {"id": "future", "reasoning": true, "efforts": ["custom"]}
            ]}
        }))
        .expect("model frame");
        let PiWireMessage::Response { response, .. } = parse_frame(&frame).expect("model response")
        else {
            panic!("expected response");
        };

        let models = response.data["models"].as_array().expect("models");
        let efforts = models
            .iter()
            .map(|model| model["efforts"].clone())
            .collect::<Vec<_>>();
        assert_eq!(
            efforts,
            [
                serde_json::json!(["off"]),
                serde_json::json!(["off", "minimal", "low", "medium", "high"]),
                serde_json::json!(["off", "low", "medium", "high", "xhigh"]),
                serde_json::json!(["custom"]),
            ]
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
