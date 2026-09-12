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
        commands: Vec<PiCommand>,
    },
    ExtensionUi(ExtensionUiRequest),
    Event(Value),
}

/// Pi-only provenance for discovering controls; it never enters the shared catalog.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(super) struct PiCommand {
    pub name: String,
    pub source: crate::agents::extensions::SlashCommandSource,
    #[serde(rename = "sourceInfo")]
    pub source_info: Option<CommandSource>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(super) struct CommandSource {
    pub path: Option<std::path::PathBuf>,
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
            let operation = response_operation(&response.command);
            let mut commands = Vec::new();
            let result = if response.success {
                let provenance = if response.command == "get_commands" {
                    serde_json::from_value(response.data["commands"].clone())
                        .map_err(|error| format!("invalid Pi command sources: {error}"))
                } else {
                    Ok(Vec::new())
                };
                provenance.and_then(|catalog| {
                    commands = catalog;
                    super::response::decode(operation, response.data)
                })
            } else {
                Err(response
                    .error
                    .unwrap_or_else(|| format!("Pi rejected {operation:?}")))
            };
            let decoded = match result {
                Ok(payload) => SessionResponse::success(response.id, payload),
                Err(error) => SessionResponse::failure(response.id, operation, error),
            };
            Ok(PiWireMessage::Response {
                command: response.command,
                response: decoded,
                commands,
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

pub(super) fn response_operation(command: &str) -> SessionOperation {
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
#[path = "wire_tests.rs"]
mod tests;
