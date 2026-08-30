use serde_json::{Value, json};

use super::process::PiRpcProcess;
use crate::{
    agents::{PiEvent, PiRequest, PiSessionTransport},
    protocol::{ExtensionUiResponse, PromptMode},
};

pub(super) fn encode_request(request: PiRequest) -> Value {
    match request {
        PiRequest::ConfigureSteering => json!({
            "type": "set_steering_mode",
            "mode": "all",
        }),
        PiRequest::LoadState => json!({"type": "get_state"}),
        PiRequest::LoadHistory => json!({"type": "get_entries"}),
        PiRequest::LoadUsage => json!({"type": "get_session_stats"}),
        PiRequest::ListModels => json!({"type": "get_available_models"}),
        PiRequest::ListReasoningLevels => {
            json!({"type": "get_available_thinking_levels"})
        }
        PiRequest::ListCommands => json!({"type": "get_commands"}),
        PiRequest::Prompt {
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
        PiRequest::Abort => json!({"type": "abort"}),
        PiRequest::Compact { instructions } => {
            optional_string("compact", "customInstructions", instructions)
        }
        PiRequest::ExportHtml { output_path } => {
            optional_string("export_html", "outputPath", output_path)
        }
        PiRequest::Rename { name } => {
            json!({"type": "set_session_name", "name": name})
        }
        PiRequest::ForkAt { entry_id } => {
            json!({"type": "fork", "entryId": entry_id})
        }
        PiRequest::SelectModel { provider, model_id } => json!({
            "type": "set_model",
            "provider": provider,
            "modelId": model_id,
        }),
        PiRequest::SelectReasoning { level } => {
            json!({"type": "set_thinking_level", "level": level})
        }
    }
}

impl PiSessionTransport for PiRpcProcess {
    fn send(&mut self, request: PiRequest) -> Result<String, String> {
        self.send_request(request)
    }

    fn respond(&mut self, response: ExtensionUiResponse) -> Result<(), String> {
        self.send_extension_response(response)
    }

    fn poll(&mut self) -> Option<PiEvent> {
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
            encode_request(PiRequest::ConfigureSteering),
            json!({"type":"set_steering_mode","mode":"all"})
        );
        assert_eq!(
            encode_request(PiRequest::Prompt {
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
            encode_request(PiRequest::Compact { instructions: None }),
            json!({"type":"compact"})
        );
    }
}
