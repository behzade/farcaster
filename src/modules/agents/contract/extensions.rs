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
    pub service_tier: Option<String>,
    #[serde(default)]
    pub service_tiers: Vec<String>,
    #[serde(default)]
    pub model: Option<Model>,
    #[serde(default)]
    pub thinking_level: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub efforts: Option<Vec<String>>,
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub data: String,
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<std::path::PathBuf>,
}

impl PromptImage {
    pub(crate) fn new(data: String, mime_type: String) -> Self {
        Self {
            data,
            mime_type,
            path: None,
        }
    }

    pub(crate) fn from_file(path: std::path::PathBuf, mime_type: String) -> Self {
        Self {
            data: String::new(),
            mime_type,
            path: Some(path),
        }
    }

    pub(crate) fn bytes(&self) -> Result<Vec<u8>, String> {
        use base64::Engine as _;
        match &self.path {
            Some(path) => std::fs::read(path)
                .map_err(|error| format!("read image {}: {error}", path.display())),
            None => base64::engine::general_purpose::STANDARD
                .decode(&self.data)
                .map_err(|error| format!("decode image: {error}")),
        }
    }

    pub(crate) fn into_inline(self) -> Result<Self, String> {
        use base64::Engine as _;
        if self.path.is_none() {
            return Ok(self);
        }
        let data = base64::engine::general_purpose::STANDARD.encode(self.bytes()?);
        Ok(Self::new(data, self.mime_type))
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
#[path = "extensions_tests.rs"]
mod tests;
