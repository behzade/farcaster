//! Pi request encoding at the backend boundary.

use serde_json::{Value, json};

use super::{BackendEvent, BackendRequest, SessionTransport};
use crate::{
    protocol::{ExtensionUiResponse, PromptMode},
    rpc_process::RpcProcess,
};

pub(super) fn encode_request(request: BackendRequest) -> Value {
    match request {
        BackendRequest::ConfigureSteering => json!({
            "type": "set_steering_mode",
            "mode": "all",
        }),
        BackendRequest::LoadState => json!({"type": "get_state"}),
        BackendRequest::LoadHistory => json!({"type": "get_entries"}),
        BackendRequest::LoadUsage => json!({"type": "get_session_stats"}),
        BackendRequest::ListModels => json!({"type": "get_available_models"}),
        BackendRequest::ListReasoningLevels => {
            json!({"type": "get_available_thinking_levels"})
        }
        BackendRequest::ListCommands => json!({"type": "get_commands"}),
        BackendRequest::Prompt {
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
        BackendRequest::Abort => json!({"type": "abort"}),
        BackendRequest::Compact { instructions } => {
            optional_string("compact", "customInstructions", instructions)
        }
        BackendRequest::ExportHtml { output_path } => {
            optional_string("export_html", "outputPath", output_path)
        }
        BackendRequest::Rename { name } => {
            json!({"type": "set_session_name", "name": name})
        }
        BackendRequest::Login { provider } => optional_string("login", "provider", provider),
        BackendRequest::SelectModel { provider, model_id } => json!({
            "type": "set_model",
            "provider": provider,
            "modelId": model_id,
        }),
        BackendRequest::SelectReasoning { level } => {
            json!({"type": "set_thinking_level", "level": level})
        }
    }
}

impl SessionTransport for RpcProcess {
    fn send(&mut self, request: BackendRequest) -> Result<String, String> {
        self.send_request(request)
    }

    fn respond(&mut self, response: ExtensionUiResponse) -> Result<(), String> {
        self.send_extension_response(response)
    }

    fn poll(&mut self) -> Option<BackendEvent> {
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
mod tests {
    use super::*;
    use crate::protocol::PromptImage;

    #[test]
    fn encodes_pi_requests_only_at_the_adapter_boundary() {
        assert_eq!(
            encode_request(BackendRequest::ConfigureSteering),
            json!({"type":"set_steering_mode","mode":"all"})
        );
        assert_eq!(
            encode_request(BackendRequest::Prompt {
                mode: PromptMode::FollowUp,
                message: "later".into(),
                images: vec![PromptImage::new("aGVsbG8=".into(), "image/png".into())],
            }),
            json!({
                "type":"follow_up",
                "message":"later",
                "images":[{"type":"image","data":"aGVsbG8=","mimeType":"image/png"}],
            })
        );
        assert_eq!(
            encode_request(BackendRequest::Compact { instructions: None }),
            json!({"type":"compact"})
        );
    }
}
