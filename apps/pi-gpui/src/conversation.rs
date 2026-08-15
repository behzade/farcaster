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
    pub latest_cache_hit_rate: Option<f64>,
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
    label: String,
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
    pub(crate) fn push_local_user(&mut self, message: String) {
        if self.items.last().is_some_and(|item| {
            item.kind == TranscriptKind::User && item.text == message && !item.is_error
        }) {
            return;
        }
        self.items.push(TranscriptItem {
            kind: TranscriptKind::User,
            label: String::new(),
            text: message,
            streaming: false,
            is_error: false,
        });
    }

    pub(crate) fn replace_history(&mut self, messages: &[Value]) {
        self.items = messages.iter().flat_map(project_message_items).collect();
        self.latest_cache_hit_rate = None;
        for message in messages {
            if message.get("role").and_then(Value::as_str) == Some("assistant") {
                self.latest_cache_hit_rate = cache_hit_rate(message);
            }
        }
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
        let mut projected = message.map(project_message_items).unwrap_or_default();
        for item in &mut projected {
            item.streaming = true;
        }
        if projected.len() == 1
            && self.items.last().is_some_and(|last| {
                last.kind == TranscriptKind::User
                    && last.text == projected[0].text
                    && !last.streaming
            })
        {
            let start = self.items.len().saturating_sub(1);
            self.items[start] = projected.remove(0);
            self.live_message = Some(LiveMessage { start, len: 1 });
            return;
        }
        let start = self.items.len();
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
            "toolcall_start" => {
                reset_partial(partial, PartialKind::ToolCall);
                partial.label = delta
                    .get("toolCall")
                    .and_then(tool_name)
                    .or_else(|| delta.get("toolName").and_then(Value::as_str))
                    .map(display_tool_name)
                    .unwrap_or_default();
            }
            "toolcall_delta" => append_delta(partial, delta),
            "toolcall_end" => {
                if let Some(tool_call) = delta.get("toolCall") {
                    partial.label = tool_name(tool_call)
                        .map(display_tool_name)
                        .unwrap_or_else(|| "Tool".into());
                    partial.value = tool_arguments(tool_call);
                }
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
                    PartialKind::Text | PartialKind::Thinking => "",
                    PartialKind::ToolCall if partial.label.is_empty() => "Tool",
                    PartialKind::ToolCall => &partial.label,
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
        if message.get("role").and_then(Value::as_str) == Some("assistant") {
            self.latest_cache_hit_rate = cache_hit_rate(message);
        }
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
        let args = event.get("args").map(readable_json).unwrap_or_default();
        self.items.push(TranscriptItem {
            kind: TranscriptKind::Tool,
            label: display_tool_name(&name),
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
    partial.label.clear();
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

fn cache_hit_rate(message: &Value) -> Option<f64> {
    let usage = message.get("usage")?;
    let input = usage.get("input").and_then(Value::as_u64)?;
    let cache_read = usage.get("cacheRead").and_then(Value::as_u64).unwrap_or(0);
    let cache_write = usage.get("cacheWrite").and_then(Value::as_u64).unwrap_or(0);
    let prompt_tokens = input.saturating_add(cache_read).saturating_add(cache_write);
    (prompt_tokens > 0).then(|| cache_read as f64 / prompt_tokens as f64 * 100.0)
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
                                String::new(),
                                block.get("text").and_then(Value::as_str)?.to_owned(),
                            ),
                            Some("thinking") => (
                                TranscriptKind::Thinking,
                                String::new(),
                                block.get("thinking").and_then(Value::as_str)?.to_owned(),
                            ),
                            Some("toolCall") => (
                                TranscriptKind::Tool,
                                tool_name(block)
                                    .map(display_tool_name)
                                    .unwrap_or_else(|| "Tool".into()),
                                tool_arguments(block),
                            ),
                            _ => return None,
                        };
                        Some(TranscriptItem {
                            kind: if is_error && kind != TranscriptKind::Tool {
                                TranscriptKind::Error
                            } else {
                                kind
                            },
                            label,
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
        "user" => (TranscriptKind::User, String::new(), true),
        "toolResult" => (
            TranscriptKind::Tool,
            message
                .get("toolName")
                .and_then(Value::as_str)
                .map(display_tool_name)
                .unwrap_or_else(|| "Tool".into()),
            true,
        ),
        "bashExecution" => (TranscriptKind::Tool, "Shell".into(), true),
        "custom" => (
            TranscriptKind::Custom,
            "Extension".into(),
            message
                .get("display")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        ),
        "branchSummary" | "compactionSummary" => (TranscriptKind::Notice, "Summary".into(), true),
        _ => return Vec::new(),
    };
    if !display {
        return Vec::new();
    }
    vec![TranscriptItem {
        kind: if is_error && kind != TranscriptKind::Tool {
            TranscriptKind::Error
        } else {
            kind
        },
        label,
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
    if let Some(text) = message
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
    {
        return text;
    }
    message
        .get("summary")
        .and_then(Value::as_str)
        .or_else(|| message.get("output").and_then(Value::as_str))
        .unwrap_or_default()
        .to_owned()
}

fn tool_name(value: &Value) -> Option<&str> {
    value
        .get("name")
        .or_else(|| value.get("toolName"))
        .and_then(Value::as_str)
}

fn tool_arguments(value: &Value) -> String {
    value
        .get("arguments")
        .map(readable_json)
        .unwrap_or_default()
}

pub(crate) fn display_tool_name(name: &str) -> String {
    let title = name
        .trim()
        .split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        "Tool".into()
    } else {
        title
    }
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
        .unwrap_or_else(|| readable_json(value))
}

fn readable_json(value: &Value) -> String {
    if let Some(raw) = value.as_str()
        && let Ok(parsed) = serde_json::from_str::<Value>(raw)
    {
        return readable_json(&parsed);
    }
    let mut output = String::new();
    write_readable_json(value, 0, &mut output);
    if output.is_empty() {
        "None".into()
    } else {
        output
    }
}

fn write_readable_json(value: &Value, depth: usize, output: &mut String) {
    match value {
        Value::Object(fields) if fields.is_empty() => output.push_str("None"),
        Value::Object(fields) => {
            for (index, (key, value)) in fields.iter().enumerate() {
                if index > 0 {
                    output.push('\n');
                }
                push_indent(output, depth);
                output.push_str(&display_field_name(key));
                output.push(':');
                if let Some(value) = readable_scalar(value) {
                    output.push(' ');
                    output.push_str(&value);
                } else {
                    output.push('\n');
                    write_readable_json(value, depth.saturating_add(1), output);
                }
            }
        }
        Value::Array(items) if items.is_empty() => output.push_str("None"),
        Value::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                if index > 0 {
                    output.push('\n');
                }
                push_indent(output, depth);
                output.push('-');
                if let Some(value) = readable_scalar(value) {
                    output.push(' ');
                    output.push_str(&value);
                } else {
                    output.push('\n');
                    write_readable_json(value, depth.saturating_add(1), output);
                }
            }
        }
        value => output.push_str(&readable_scalar(value).unwrap_or_default()),
    }
}

fn readable_scalar(value: &Value) -> Option<String> {
    match value {
        Value::Null => Some("None".into()),
        Value::Bool(value) => Some(if *value { "Yes" } else { "No" }.into()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => Some(value.clone()),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn push_indent(output: &mut String, depth: usize) {
    output.extend(std::iter::repeat_n(' ', depth.saturating_mul(2)));
}

fn display_field_name(name: &str) -> String {
    let mut words = String::with_capacity(name.len());
    let mut previous_was_lowercase = false;
    for character in name.chars() {
        if character == '_' {
            if !words.ends_with(' ') {
                words.push(' ');
            }
            previous_was_lowercase = false;
        } else {
            if character.is_uppercase() && previous_was_lowercase {
                words.push(' ');
            }
            words.extend(character.to_lowercase());
            previous_was_lowercase = character.is_lowercase();
        }
    }
    let mut characters = words.trim().chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
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
