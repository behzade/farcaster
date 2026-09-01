use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::json;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum BackgroundJobState {
    Starting,
    Running,
    Completed,
    Exited,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BackgroundJob {
    pub name: String,
    pub command: String,
    pub state: BackgroundJobState,
    #[serde(default)]
    pub exit_code: Option<i32>,
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct Model {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default, rename = "contextWindow")]
    pub context_window: u64,
    #[serde(default)]
    pub reasoning: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct AgentMode {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct SlashCommand {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub source: SlashCommandSource,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SlashCommandSource {
    Extension,
    Prompt,
    Skill,
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

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "method")]
pub(crate) enum ExtensionUiRequest {
    #[serde(rename = "select")]
    Select {
        id: String,
        title: String,
        options: Vec<String>,
        timeout: Option<u64>,
    },
    #[serde(rename = "confirm")]
    Confirm {
        id: String,
        title: String,
        message: String,
        timeout: Option<u64>,
    },
    #[serde(rename = "input")]
    Input {
        id: String,
        title: String,
        placeholder: Option<String>,
        timeout: Option<u64>,
    },
    #[serde(rename = "editor")]
    Editor {
        id: String,
        title: String,
        prefill: Option<String>,
    },
    #[serde(rename = "notify")]
    Notify {
        id: String,
        message: String,
        #[serde(default, rename = "notifyType")]
        tone: NotifyTone,
    },
    #[serde(rename = "setStatus")]
    SetStatus {
        id: String,
        #[serde(rename = "statusKey")]
        key: String,
        #[serde(rename = "statusText")]
        text: Option<String>,
    },
    #[serde(rename = "setWidget")]
    SetWidget {
        id: String,
        #[serde(rename = "widgetKey")]
        key: String,
        #[serde(rename = "widgetLines")]
        lines: Option<Vec<String>>,
        #[serde(default, rename = "widgetPlacement")]
        placement: WidgetPlacement,
    },
    #[serde(rename = "setTitle")]
    SetTitle { id: String, title: String },
    #[serde(rename = "set_editor_text")]
    SetEditorText { id: String, text: String },
    #[serde(skip)]
    Unknown { id: Option<String>, method: String },
}

impl ExtensionUiRequest {
    pub(crate) fn gpui_system_notification(&self) -> Option<(&str, &str)> {
        let Self::Notify { message, .. } = self else {
            return None;
        };
        let payload = message
            .strip_prefix("\u{1f}farcaster-notification\u{1f}")
            .or_else(|| message.strip_prefix("\u{1f}pi-gpui-notification\u{1f}"))?;
        payload.split_once('\u{1f}')
    }

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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum NotifyTone {
    Warning,
    Error,
    #[default]
    #[serde(other)]
    Info,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WidgetPlacement {
    BelowEditor,
    #[default]
    #[serde(other)]
    AboveEditor,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpui_notification_transport_separates_title_and_body() {
        let request = ExtensionUiRequest::Notify {
            id: "notification".into(),
            message: "\u{1f}farcaster-notification\u{1f}Pi finished\u{1f}Done".into(),
            tone: NotifyTone::Info,
        };
        assert_eq!(
            request.gpui_system_notification(),
            Some(("Pi finished", "Done"))
        );
        let legacy = ExtensionUiRequest::Notify {
            id: "notification".into(),
            message: "\u{1f}pi-gpui-notification\u{1f}Pi finished\u{1f}Done".into(),
            tone: NotifyTone::Info,
        };
        assert_eq!(
            legacy.gpui_system_notification(),
            Some(("Pi finished", "Done"))
        );
    }

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
                serde_json::from_str::<ExtensionUiRequest>(frame).expect("extension request"),
                expected
            );
        }
    }

    #[test]
    fn rejects_malformed_known_extension_requests() {
        for frame in [
            r#"{"type":"extension_ui_request","method":"confirm","title":"T","message":"M"}"#,
            r#"{"type":"extension_ui_request","id":"1","method":"select","title":"T","options":["a",1]}"#,
        ] {
            assert!(
                serde_json::from_str::<ExtensionUiRequest>(frame).is_err(),
                "accepted {frame}"
            );
        }
    }

    #[test]
    fn unknown_extension_enum_values_use_protocol_defaults() {
        assert_eq!(
            serde_json::from_str::<ExtensionUiRequest>(
                r#"{"type":"extension_ui_request","id":"1","method":"notify","message":"M","notifyType":"future"}"#
            ).expect("notification"),
            ExtensionUiRequest::Notify {
                id: "1".into(),
                message: "M".into(),
                tone: NotifyTone::Info,
            }
        );
        assert_eq!(
            serde_json::from_str::<ExtensionUiRequest>(
                r#"{"type":"extension_ui_request","id":"2","method":"setWidget","widgetKey":"k","widgetPlacement":"future"}"#
            ).expect("widget"),
            ExtensionUiRequest::SetWidget {
                id: "2".into(),
                key: "k".into(),
                lines: None,
                placement: WidgetPlacement::AboveEditor,
            }
        );
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
    fn slash_commands_decode_without_depending_on_source_metadata_shape() {
        let command = serde_json::from_value::<SlashCommand>(json!({
            "name": "reload",
            "description": "Reload extensions",
            "source": "extension",
            "sourceInfo": {"scope": "project"}
        }))
        .expect("slash command should decode");
        assert_eq!(
            command,
            SlashCommand {
                name: "reload".into(),
                description: Some("Reload extensions".into()),
                source: SlashCommandSource::Extension,
            }
        );
    }
}
