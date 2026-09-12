use serde_json::{Value, json};

use super::process::PiRpcProcess;
use crate::{
    agents::extensions::{ExtensionUiResponse, PromptMode},
    agents::{SessionCommand, SessionEvent, SessionTransport},
};

pub(super) fn encode_request(request: SessionCommand) -> Result<Value, String> {
    Ok(match request {
        SessionCommand::SelectServiceTier { .. } => {
            return Err("Pi does not advertise service tier selection".into());
        }
        SessionCommand::ConfigureSteering => json!({
            "type": "set_steering_mode",
            "mode": "all",
        }),
        SessionCommand::LoadState => json!({"type": "get_state"}),
        SessionCommand::LoadHistory => json!({"type": "get_entries"}),
        SessionCommand::LoadUsage => json!({"type": "get_session_stats"}),
        SessionCommand::ListModels => json!({"type": "get_available_models"}),
        SessionCommand::ListReasoningLevels => {
            json!({"type": "get_available_thinking_levels"})
        }
        SessionCommand::ListModes => json!({"type": "get_modes"}),
        SessionCommand::ListCommands => json!({"type": "get_commands"}),
        SessionCommand::Prompt {
            mode,
            message,
            images,
        } => {
            let kind = match mode {
                PromptMode::Normal => "prompt",
                PromptMode::Steer => "steer",
                PromptMode::FollowUp => "follow_up",
            };
            let mut value = json!({"type": kind, "message": message});
            if !images.is_empty() {
                value["images"] = json!(images);
            }
            value
        }
        // Pi's native abort applies queued steering. PiRpcProcess intercepts a
        // user Abort and retires the process so queued work cannot continue.
        SessionCommand::ApplySteering | SessionCommand::Abort => json!({"type": "abort"}),
        SessionCommand::Compact { instructions } => {
            optional_string("compact", "customInstructions", instructions)
        }
        SessionCommand::ExportHtml { output_path } => {
            optional_string("export_html", "outputPath", output_path)
        }
        SessionCommand::Rename { name } => {
            json!({"type": "set_session_name", "name": name})
        }
        SessionCommand::ForkAt { entry_id } => {
            json!({"type": "fork", "entryId": entry_id})
        }
        SessionCommand::SelectModel { provider, model_id } => json!({
            "type": "set_model",
            "provider": provider,
            "modelId": model_id,
        }),
        SessionCommand::SelectReasoning { level } => {
            json!({"type": "set_thinking_level", "level": level})
        }
        SessionCommand::SelectMode { mode } => json!({"type": "set_mode", "mode": mode}),
    })
}

impl SessionTransport for PiRpcProcess {
    fn sandbox_adapter(&self) -> Option<&str> {
        self.sandbox_adapter_id()
    }
    fn sandbox_mode(&self) -> Option<crate::agents::HarnessAccessMode> {
        self.confirmed_sandbox_mode()
    }
    fn send(&mut self, request: SessionCommand) -> Result<String, String> {
        self.send_request(request)
    }

    fn respond(&mut self, response: ExtensionUiResponse) -> Result<(), String> {
        self.send_extension_response(response)
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.try_next()
    }

    fn close(&mut self) -> Result<(), String> {
        self.terminate()
    }
}

fn optional_string(kind: &str, field: &str, value: Option<String>) -> Value {
    let mut request = serde_json::Map::from_iter([("type".into(), Value::String(kind.into()))]);
    if let Some(value) = value {
        request.insert(field.into(), Value::String(value));
    }
    Value::Object(request)
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
