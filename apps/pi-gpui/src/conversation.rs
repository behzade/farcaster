//! Pure transcript and live-run reducer.

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

const MAX_DIAGNOSTICS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TranscriptKind {
    User,
    Assistant,
    Thinking,
    Tool,
    Error,
    Notice,
    Custom,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TranscriptItem {
    pub kind: TranscriptKind,
    pub label: String,
    pub text: String,
    pub streaming: bool,
    pub is_error: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct QueueState {
    pub steering: Vec<String>,
    pub follow_up: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ConversationState {
    pub items: Vec<TranscriptItem>,
    pub queue: QueueState,
    pub running: bool,
    pub settled: bool,
    pub compacting: bool,
    pub retrying: bool,
    pub diagnostics: Vec<String>,
    live_message: Option<LiveMessage>,
    content: BTreeMap<usize, PartialContent>,
    tools: HashMap<String, usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LiveMessage {
    start: usize,
    len: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct PartialContent {
    kind: PartialKind,
    value: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PartialKind {
    #[default]
    Text,
    Thinking,
    ToolCall,
}

impl ConversationState {
    pub(crate) fn replace_history(&mut self, messages: &[Value]) {
        self.items = messages.iter().flat_map(project_message_items).collect();
        self.live_message = None;
        self.content.clear();
        self.tools.clear();
    }

    pub(crate) fn reduce(&mut self, event: &Value) {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match kind {
            "agent_start" => {
                self.running = true;
                self.settled = false;
            }
            "agent_end" => {
                if event.get("willRetry").and_then(Value::as_bool) == Some(true) {
                    self.retrying = true;
                }
            }
            "agent_settled" => {
                self.running = false;
                self.settled = true;
                self.retrying = false;
                self.compacting = false;
            }
            "message_start" => self.start_message(event.get("message")),
            "message_update" => self.update_message(event.get("assistantMessageEvent")),
            "message_end" => self.end_message(event.get("message")),
            "tool_execution_start" => self.start_tool(event),
            "tool_execution_update" => self.update_tool(event),
            "tool_execution_end" => self.end_tool(event),
            "queue_update" => {
                self.queue.steering = strings(event.get("steering"));
                self.queue.follow_up = strings(event.get("followUp"));
            }
            "compaction_start" => {
                self.compacting = true;
                self.notice(format!(
                    "Compacting context ({})",
                    text_field(event, "reason")
                ));
            }
            "compaction_end" => {
                self.compacting = false;
                let message = if event.get("aborted").and_then(Value::as_bool) == Some(true) {
                    "Compaction aborted".to_owned()
                } else if let Some(error) = event.get("errorMessage").and_then(Value::as_str) {
                    format!("Compaction failed: {error}")
                } else {
                    "Context compacted".to_owned()
                };
                self.notice(message);
            }
            "auto_retry_start"
            | "summarization_retry_scheduled"
            | "summarization_retry_attempt_start" => {
                self.retrying = true;
                self.notice(retry_notice(event));
            }
            "auto_retry_end" | "summarization_retry_finished" => {
                self.retrying = false;
                self.notice(retry_notice(event));
            }
            "extension_error" => self.push_extension_error(text_field(event, "error")),
            "turn_start" | "turn_end" => {}
            unknown => self.diagnostic(format!("Unknown RPC event: {unknown}")),
        }
    }

    pub(crate) fn push_transport_error(&mut self, message: String) {
        self.running = false;
        self.items.push(TranscriptItem {
            kind: TranscriptKind::Error,
            label: "Connection error".into(),
            text: message,
            streaming: false,
            is_error: true,
        });
    }

    pub(crate) fn push_extension_error(&mut self, message: String) {
        self.items.push(TranscriptItem {
            kind: TranscriptKind::Error,
            label: "Extension error".into(),
            text: message,
            streaming: false,
            is_error: true,
        });
    }

    pub(crate) fn push_local_error(&mut self, label: &str, message: String) {
        self.items.push(TranscriptItem {
            kind: TranscriptKind::Error,
            label: label.into(),
            text: message,
            streaming: false,
            is_error: true,
        });
    }

    fn start_message(&mut self, message: Option<&Value>) {
        self.content.clear();
        let start = self.items.len();
        let mut projected = message.map(project_message_items).unwrap_or_default();
        for item in &mut projected {
            item.streaming = true;
        }
        let len = projected.len();
        self.items.extend(projected);
        self.live_message = Some(LiveMessage { start, len });
    }

    fn update_message(&mut self, delta: Option<&Value>) {
        let Some(delta) = delta else { return };
        let Some(content_index) = delta
            .get("contentIndex")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
        else {
            return;
        };
        let delta_type = delta
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let partial = self.content.entry(content_index).or_default();
        match delta_type {
            "text_start" => reset_partial(partial, PartialKind::Text),
            "text_delta" => append_delta(partial, delta),
            "text_end" => finish_content(partial, delta),
            "thinking_start" => reset_partial(partial, PartialKind::Thinking),
            "thinking_delta" => append_delta(partial, delta),
            "thinking_end" => finish_content(partial, delta),
            "toolcall_start" => reset_partial(partial, PartialKind::ToolCall),
            "toolcall_delta" => append_delta(partial, delta),
            "toolcall_end" => {
                partial.value = delta
                    .get("toolCall")
                    .map(tool_call_text)
                    .unwrap_or_else(|| partial.value.clone());
            }
            _ => return,
        }
        self.refresh_live_projection();
    }

    fn refresh_live_projection(&mut self) {
        let Some(live) = self.live_message else {
            return;
        };
        let projected = self
            .content
            .values()
            .map(|partial| TranscriptItem {
                kind: match partial.kind {
                    PartialKind::Text => TranscriptKind::Assistant,
                    PartialKind::Thinking => TranscriptKind::Thinking,
                    PartialKind::ToolCall => TranscriptKind::Tool,
                },
                label: match partial.kind {
                    PartialKind::Text => "Pi",
                    PartialKind::Thinking => "Thinking",
                    PartialKind::ToolCall => "Tool call",
                }
                .into(),
                text: partial.value.clone(),
                streaming: true,
                is_error: false,
            })
            .collect::<Vec<_>>();
        let len = projected.len();
        self.items
            .splice(live.start..live.start + live.len, projected);
        self.live_message = Some(LiveMessage {
            start: live.start,
            len,
        });
    }

    fn end_message(&mut self, message: Option<&Value>) {
        let Some(message) = message else { return };
        let final_items = project_message_items(message);
        if let Some(live) = self.live_message.take() {
            self.items
                .splice(live.start..live.start + live.len, final_items);
        } else {
            self.items.extend(final_items);
        }
        self.content.clear();
    }

    fn start_tool(&mut self, event: &Value) {
        let id = text_field(event, "toolCallId");
        let name = text_field(event, "toolName");
        let args = event.get("args").map(compact_json).unwrap_or_default();
        self.items.push(TranscriptItem {
            kind: TranscriptKind::Tool,
            label: format!("Running {name}"),
            text: args,
            streaming: true,
            is_error: false,
        });
        self.tools.insert(id, self.items.len() - 1);
    }

    fn update_tool(&mut self, event: &Value) {
        let id = text_field(event, "toolCallId");
        if let Some(index) = self.tools.get(&id).copied()
            && let Some(item) = self.items.get_mut(index)
        {
            item.text = event
                .get("partialResult")
                .map(result_text)
                .unwrap_or_default();
        }
    }

    fn end_tool(&mut self, event: &Value) {
        let id = text_field(event, "toolCallId");
        let is_error = event
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(index) = self.tools.remove(&id)
            && let Some(item) = self.items.get_mut(index)
        {
            item.text = event.get("result").map(result_text).unwrap_or_default();
            item.streaming = false;
            item.is_error = is_error;
            item.label = if is_error {
                "Tool failed"
            } else {
                "Tool finished"
            }
            .into();
        }
    }

    fn notice(&mut self, text: String) {
        self.items.push(TranscriptItem {
            kind: TranscriptKind::Notice,
            label: "Run".into(),
            text,
            streaming: false,
            is_error: false,
        });
    }

    fn diagnostic(&mut self, text: String) {
        self.diagnostics.push(text);
        if self.diagnostics.len() > MAX_DIAGNOSTICS {
            self.diagnostics.remove(0);
        }
    }
}

fn reset_partial(partial: &mut PartialContent, kind: PartialKind) {
    partial.kind = kind;
    partial.value.clear();
}

fn append_delta(partial: &mut PartialContent, delta: &Value) {
    partial.value.push_str(
        delta
            .get("delta")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
}

fn finish_content(partial: &mut PartialContent, delta: &Value) {
    partial.value = delta
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or(&partial.value)
        .to_owned();
}

fn project_message_items(message: &Value) -> Vec<TranscriptItem> {
    let Some(role) = message.get("role").and_then(Value::as_str) else {
        return Vec::new();
    };
    let is_error = message
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || message.get("stopReason").and_then(Value::as_str) == Some("error");
    if role == "assistant" {
        return message
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|block| {
                        let (kind, label, text) = match block.get("type").and_then(Value::as_str) {
                            Some("text") => (
                                TranscriptKind::Assistant,
                                "Pi",
                                block.get("text").and_then(Value::as_str)?.to_owned(),
                            ),
                            Some("thinking") => (
                                TranscriptKind::Thinking,
                                "Thinking",
                                block.get("thinking").and_then(Value::as_str)?.to_owned(),
                            ),
                            Some("toolCall") => {
                                (TranscriptKind::Tool, "Tool call", tool_call_text(block))
                            }
                            _ => return None,
                        };
                        Some(TranscriptItem {
                            kind: if is_error {
                                TranscriptKind::Error
                            } else {
                                kind
                            },
                            label: label.into(),
                            text,
                            streaming: false,
                            is_error,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
    }
    let (kind, label, display) = match role {
        "user" => (TranscriptKind::User, "You", true),
        "toolResult" => (TranscriptKind::Tool, "Tool result", true),
        "bashExecution" => (TranscriptKind::Tool, "Shell", true),
        "custom" => (
            TranscriptKind::Custom,
            "Extension",
            message
                .get("display")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        ),
        "branchSummary" | "compactionSummary" => (TranscriptKind::Notice, "Summary", true),
        _ => return Vec::new(),
    };
    if !display {
        return Vec::new();
    }
    vec![TranscriptItem {
        kind: if is_error {
            TranscriptKind::Error
        } else {
            kind
        },
        label: label.into(),
        text: message_text(message),
        streaming: false,
        is_error,
    }]
}

fn message_text(message: &Value) -> String {
    if let Some(content) = message.get("content")
        && let Some(text) = content.as_str()
    {
        return text.to_owned();
    }
    message
        .get("summary")
        .and_then(Value::as_str)
        .or_else(|| message.get("output").and_then(Value::as_str))
        .unwrap_or_default()
        .to_owned()
}

fn tool_call_text(value: &Value) -> String {
    let name = value.get("name").and_then(Value::as_str).unwrap_or("tool");
    let args = value.get("arguments").map(compact_json).unwrap_or_default();
    format!("{name} {args}")
}

fn result_text(value: &Value) -> String {
    value
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| compact_json(value))
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}
fn text_field(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
fn retry_notice(event: &Value) -> String {
    let kind = text_field(event, "type");
    let attempt = event.get("attempt").and_then(Value::as_u64);
    attempt.map_or(kind.clone(), |attempt| {
        format!("{kind} · attempt {attempt}")
    })
}

#[cfg(test)]
#[path = "conversation_tests.rs"]
mod tests;
