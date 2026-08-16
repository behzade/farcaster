//! Pi RPC wire DTOs. Unknown events and fields stay available as JSON values.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct RpcResponse {
    pub id: Option<String>,
    pub command: String,
    pub success: bool,
    #[serde(default)]
    pub data: Value,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionState {
    #[serde(default)]
    pub model: Option<Model>,
    pub thinking_level: String,
    pub is_streaming: bool,
    pub is_compacting: bool,
    #[serde(default)]
    pub session_file: Option<String>,
    pub session_id: String,
    #[serde(default)]
    pub session_name: Option<String>,
    pub auto_compaction_enabled: bool,
    pub message_count: usize,
    pub pending_message_count: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct Model {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default)]
    pub reasoning: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptMode {
    Normal,
    Steer,
    FollowUp,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename = "image", rename_all = "camelCase", tag = "type")]
pub(crate) struct PromptImage {
    pub data: String,
    pub mime_type: String,
}

impl PromptImage {
    pub(crate) fn new(data: String, mime_type: String) -> Self {
        Self { data, mime_type }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WireMessage {
    Response(RpcResponse),
    ExtensionUi(ExtensionUiRequest),
    Event(Value),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExtensionUiRequest {
    Select {
        id: String,
        title: String,
        options: Vec<String>,
        timeout: Option<u64>,
    },
    Confirm {
        id: String,
        title: String,
        message: String,
        timeout: Option<u64>,
    },
    Input {
        id: String,
        title: String,
        placeholder: Option<String>,
        timeout: Option<u64>,
    },
    Editor {
        id: String,
        title: String,
        prefill: Option<String>,
    },
    Notify {
        id: String,
        message: String,
        tone: NotifyTone,
    },
    SetStatus {
        id: String,
        key: String,
        text: Option<String>,
    },
    SetWidget {
        id: String,
        key: String,
        lines: Option<Vec<String>>,
        placement: WidgetPlacement,
    },
    SetTitle {
        id: String,
        title: String,
    },
    SetEditorText {
        id: String,
        text: String,
    },
    Unknown {
        id: Option<String>,
        method: String,
    },
}

impl ExtensionUiRequest {
    pub(crate) fn dialog_id(&self) -> Option<&str> {
        match self {
            Self::Select { id, .. }
            | Self::Confirm { id, .. }
            | Self::Input { id, .. }
            | Self::Editor { id, .. } => Some(id),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum NotifyTone {
    #[default]
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum WidgetPlacement {
    #[default]
    AboveEditor,
    BelowEditor,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ExtensionUiResponse {
    #[serde(rename = "extension_ui_response")]
    Value { id: String, value: String },
    #[serde(rename = "extension_ui_response")]
    Confirmed { id: String, confirmed: bool },
    #[serde(rename = "extension_ui_response")]
    Cancelled { id: String, cancelled: bool },
}

pub(crate) fn command(command_type: &str) -> Value {
    json!({ "type": command_type })
}

pub(crate) fn prompt_command(mode: PromptMode, message: String, images: Vec<PromptImage>) -> Value {
    let mut command = match mode {
        PromptMode::Normal => json!({"type":"prompt", "message":message}),
        PromptMode::Steer => json!({"type":"steer", "message":message}),
        PromptMode::FollowUp => json!({"type":"follow_up", "message":message}),
    };
    if !images.is_empty() {
        command["images"] = json!(images);
    }
    command
}

pub(crate) fn parse_frame(frame: &[u8]) -> Result<WireMessage, String> {
    let value: Value =
        serde_json::from_slice(frame).map_err(|error| format!("malformed JSON frame: {error}"))?;
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return Err("JSON frame has no string type".to_owned());
    };
    match kind {
        "response" => serde_json::from_value(value)
            .map(WireMessage::Response)
            .map_err(|error| format!("invalid response frame: {error}")),
        "extension_ui_request" => Ok(WireMessage::ExtensionUi(parse_extension_request(&value))),
        _ => Ok(WireMessage::Event(value)),
    }
}

fn string(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn parse_extension_request(value: &Value) -> ExtensionUiRequest {
    let id = string(value, "id");
    let method = string(value, "method");
    let timeout = value.get("timeout").and_then(Value::as_u64);
    match method.as_str() {
        "select" => ExtensionUiRequest::Select {
            id,
            title: string(value, "title"),
            options: value
                .get("options")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            timeout,
        },
        "confirm" => ExtensionUiRequest::Confirm {
            id,
            title: string(value, "title"),
            message: string(value, "message"),
            timeout,
        },
        "input" => ExtensionUiRequest::Input {
            id,
            title: string(value, "title"),
            placeholder: value
                .get("placeholder")
                .and_then(Value::as_str)
                .map(str::to_owned),
            timeout,
        },
        "editor" => ExtensionUiRequest::Editor {
            id,
            title: string(value, "title"),
            prefill: value
                .get("prefill")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        "notify" => ExtensionUiRequest::Notify {
            id,
            message: string(value, "message"),
            tone: match value.get("notifyType").and_then(Value::as_str) {
                Some("warning") => NotifyTone::Warning,
                Some("error") => NotifyTone::Error,
                _ => NotifyTone::Info,
            },
        },
        "setStatus" => ExtensionUiRequest::SetStatus {
            id,
            key: string(value, "statusKey"),
            text: value
                .get("statusText")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        "setWidget" => ExtensionUiRequest::SetWidget {
            id,
            key: string(value, "widgetKey"),
            lines: value
                .get("widgetLines")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                }),
            placement: if value.get("widgetPlacement").and_then(Value::as_str)
                == Some("belowEditor")
            {
                WidgetPlacement::BelowEditor
            } else {
                WidgetPlacement::AboveEditor
            },
        },
        "setTitle" => ExtensionUiRequest::SetTitle {
            id,
            title: string(value, "title"),
        },
        "set_editor_text" => ExtensionUiRequest::SetEditorText {
            id,
            text: string(value, "text"),
        },
        _ => ExtensionUiRequest::Unknown {
            id: (!id.is_empty()).then_some(id),
            method,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_every_extension_ui_request_exactly() {
        let cases = [
            (
                r#"{"type":"extension_ui_request","id":"1","method":"select","title":"T","options":["a"],"timeout":10}"#,
                ExtensionUiRequest::Select {
                    id: "1".into(),
                    title: "T".into(),
                    options: vec!["a".into()],
                    timeout: Some(10),
                },
            ),
            (
                r#"{"type":"extension_ui_request","id":"2","method":"confirm","title":"T","message":"M","timeout":11}"#,
                ExtensionUiRequest::Confirm {
                    id: "2".into(),
                    title: "T".into(),
                    message: "M".into(),
                    timeout: Some(11),
                },
            ),
            (
                r#"{"type":"extension_ui_request","id":"3","method":"input","title":"T","placeholder":"P","timeout":12}"#,
                ExtensionUiRequest::Input {
                    id: "3".into(),
                    title: "T".into(),
                    placeholder: Some("P".into()),
                    timeout: Some(12),
                },
            ),
            (
                r#"{"type":"extension_ui_request","id":"4","method":"editor","title":"T","prefill":"P"}"#,
                ExtensionUiRequest::Editor {
                    id: "4".into(),
                    title: "T".into(),
                    prefill: Some("P".into()),
                },
            ),
            (
                r#"{"type":"extension_ui_request","id":"5","method":"notify","message":"M","notifyType":"error"}"#,
                ExtensionUiRequest::Notify {
                    id: "5".into(),
                    message: "M".into(),
                    tone: NotifyTone::Error,
                },
            ),
            (
                r#"{"type":"extension_ui_request","id":"6","method":"setStatus","statusKey":"k"}"#,
                ExtensionUiRequest::SetStatus {
                    id: "6".into(),
                    key: "k".into(),
                    text: None,
                },
            ),
            (
                r#"{"type":"extension_ui_request","id":"7","method":"setWidget","widgetKey":"k","widgetLines":["x"],"widgetPlacement":"belowEditor"}"#,
                ExtensionUiRequest::SetWidget {
                    id: "7".into(),
                    key: "k".into(),
                    lines: Some(vec!["x".into()]),
                    placement: WidgetPlacement::BelowEditor,
                },
            ),
            (
                r#"{"type":"extension_ui_request","id":"8","method":"setTitle","title":"T"}"#,
                ExtensionUiRequest::SetTitle {
                    id: "8".into(),
                    title: "T".into(),
                },
            ),
            (
                r#"{"type":"extension_ui_request","id":"9","method":"set_editor_text","text":"x"}"#,
                ExtensionUiRequest::SetEditorText {
                    id: "9".into(),
                    text: "x".into(),
                },
            ),
        ];
        for (frame, expected) in cases {
            assert_eq!(
                parse_frame(frame.as_bytes()),
                Ok(WireMessage::ExtensionUi(expected))
            );
        }
    }

    #[test]
    fn serializes_all_response_shapes() -> Result<(), serde_json::Error> {
        let values = [
            ExtensionUiResponse::Value {
                id: "1".into(),
                value: "a".into(),
            },
            ExtensionUiResponse::Confirmed {
                id: "2".into(),
                confirmed: true,
            },
            ExtensionUiResponse::Cancelled {
                id: "3".into(),
                cancelled: true,
            },
        ];
        let encoded = values
            .into_iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(
            encoded[0],
            json!({"type":"extension_ui_response","id":"1","value":"a"})
        );
        assert_eq!(
            encoded[1],
            json!({"type":"extension_ui_response","id":"2","confirmed":true})
        );
        assert_eq!(
            encoded[2],
            json!({"type":"extension_ui_response","id":"3","cancelled":true})
        );
        Ok(())
    }

    #[test]
    fn composer_commands_match_the_rpc_contract() {
        assert_eq!(
            prompt_command(PromptMode::Normal, "n".into(), Vec::new()),
            json!({"type":"prompt","message":"n"})
        );
        assert_eq!(
            prompt_command(PromptMode::Steer, "s".into(), Vec::new()),
            json!({"type":"steer","message":"s"})
        );
        assert_eq!(
            prompt_command(PromptMode::FollowUp, "f".into(), Vec::new()),
            json!({"type":"follow_up","message":"f"})
        );
        assert_eq!(
            prompt_command(
                PromptMode::Normal,
                "image".into(),
                vec![PromptImage::new("aGVsbG8=".into(), "image/png".into())],
            ),
            json!({
                "type":"prompt",
                "message":"image",
                "images":[{"type":"image","data":"aGVsbG8=","mimeType":"image/png"}]
            })
        );
        for kind in [
            "abort",
            "new_session",
            "get_state",
            "get_messages",
            "get_session_stats",
            "get_available_models",
            "get_available_thinking_levels",
            "compact",
            "set_auto_compaction",
            "set_auto_retry",
            "abort_retry",
            "get_commands",
        ] {
            assert_eq!(command(kind), json!({"type":kind}));
        }
    }

    #[test]
    fn malformed_json_is_reported() {
        assert!(parse_frame(b"{").is_err());
    }
}
