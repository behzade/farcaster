//! Pure transcript and live-run reducer.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use serde_json::Value;

const MAX_DIAGNOSTICS: usize = 32;
const STREAM_CHUNK_BYTES: usize = 2 * 1024;
const STREAM_TAIL_MAX_BYTES: usize = STREAM_CHUNK_BYTES;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditDiffFormat {
    Unnumbered,
    Numbered,
}

#[derive(Clone, Debug)]
pub(crate) enum ToolPresentation {
    Edit {
        path: String,
        diff: Option<String>,
        format: EditDiffFormat,
        prepared: Arc<std::sync::OnceLock<crate::tool_changes::PreparedToolChange>>,
    },
    Write {
        path: String,
        content: String,
        prepared: Arc<std::sync::OnceLock<crate::tool_changes::PreparedToolChange>>,
    },
}

impl PartialEq for ToolPresentation {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Edit {
                    path, diff, format, ..
                },
                Self::Edit {
                    path: other_path,
                    diff: other_diff,
                    format: other_format,
                    ..
                },
            ) => path == other_path && diff == other_diff && format == other_format,
            (
                Self::Write { path, content, .. },
                Self::Write {
                    path: other_path,
                    content: other_content,
                    ..
                },
            ) => path == other_path && content == other_content,
            _ => false,
        }
    }
}

impl Eq for ToolPresentation {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TranscriptItem {
    pub kind: TranscriptKind,
    pub label: String,
    pub text: String,
    pub stream_chunks: Arc<Vec<Arc<str>>>,
    pub streaming: bool,
    pub is_error: bool,
    pub tool_call_id: Option<String>,
    pub tool_output: String,
    pub tool_presentation: Option<ToolPresentation>,
}

impl TranscriptItem {
    pub(crate) fn complete_text(&self) -> String {
        if self.stream_chunks.is_empty() {
            return self.text.clone();
        }
        let mut text = String::with_capacity(
            self.stream_chunks
                .iter()
                .map(|chunk| chunk.len())
                .sum::<usize>()
                + self.text.len(),
        );
        for chunk in self.stream_chunks.iter() {
            text.push_str(chunk);
        }
        text.push_str(&self.text);
        text
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct QueueState {
    pub steering: Vec<String>,
    pub follow_up: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ConversationState {
    pub items: Vec<Arc<TranscriptItem>>,
    pub queue: QueueState,
    pub running: bool,
    pub settled: bool,
    pub compacting: bool,
    pub retrying: bool,
    pub average_cache_hit_rate: Option<f64>,
    pub diagnostics: Vec<String>,
    cache_hit_rate_sum: f64,
    cache_hit_rate_count: usize,
    live_message: Option<LiveMessage>,
    content: BTreeMap<usize, PartialContent>,
    dirty_content: std::collections::BTreeSet<usize>,
    projected_content: std::collections::BTreeSet<usize>,
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
    chunks: Arc<Vec<Arc<str>>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PartialKind {
    #[default]
    Text,
    Thinking,
    ToolCall,
}

impl ConversationState {
    pub(crate) fn push_local_user(
        &mut self,
        message: String,
        image_count: usize,
    ) -> Arc<TranscriptItem> {
        let item = Arc::new(TranscriptItem {
            kind: TranscriptKind::User,
            label: String::new(),
            text: user_message_text(&message, image_count),
            stream_chunks: Arc::default(),
            streaming: false,
            is_error: false,
            tool_call_id: None,
            tool_output: String::new(),
            tool_presentation: None,
        });
        self.items.push(item.clone());
        item
    }

    pub(crate) fn rollback_local_user(&mut self, optimistic: &Arc<TranscriptItem>) -> bool {
        let Some(index) = self
            .items
            .iter()
            .rposition(|item| Arc::ptr_eq(item, optimistic))
        else {
            return false;
        };
        self.items.remove(index);
        true
    }

    pub(crate) fn replace_history(&mut self, messages: &[Value]) {
        self.items.clear();
        self.average_cache_hit_rate = None;
        self.cache_hit_rate_sum = 0.0;
        self.cache_hit_rate_count = 0;
        for message in messages {
            if message.get("role").and_then(Value::as_str) == Some("assistant") {
                self.record_cache_hit_rate(message);
            }
            self.project_history_message(message);
        }
        self.live_message = None;
        self.content.clear();
        self.dirty_content.clear();
        self.projected_content.clear();
        self.tools.clear();
    }

    fn project_history_message(&mut self, message: &Value) {
        if message.get("role").and_then(Value::as_str) == Some("toolResult") {
            let id = message
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !id.is_empty()
                && let Some(item) = self.items.iter_mut().rev().find(|item| {
                    item.kind == TranscriptKind::Tool && item.tool_call_id.as_deref() == Some(id)
                })
            {
                apply_tool_result(Arc::make_mut(item), message, true);
                return;
            }
        }
        let mut projected = project_message_items(message);
        if message.get("role").and_then(Value::as_str) == Some("toolResult") {
            for item in &mut projected {
                item.tool_output = std::mem::take(&mut item.text);
            }
        }
        self.items.extend(projected.into_iter().map(Arc::new));
    }

    pub(crate) fn reduce(&mut self, event: &Value) -> Option<usize> {
        self.reduce_with_projection(event, true)
    }

    pub(crate) fn reduce_deferred(&mut self, event: &Value) -> Option<usize> {
        self.reduce_with_projection(event, false)
    }

    fn reduce_with_projection(&mut self, event: &Value, project_live: bool) -> Option<usize> {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let previous_len = self.items.len();
        let previous_live_start = self.live_message.map(|live| live.start);
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
            "message_update" => {
                self.update_message(event.get("assistantMessageEvent"), project_live)
            }
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
        match kind {
            "message_start" => Some(previous_len.saturating_sub(1)),
            "message_update" | "message_end" => previous_live_start,
            "tool_execution_start" | "tool_execution_update" | "tool_execution_end" => event
                .get("toolCallId")
                .and_then(Value::as_str)
                .and_then(|id| self.tools.get(id).copied())
                .or(Some(previous_len)),
            "compaction_start"
            | "compaction_end"
            | "auto_retry_start"
            | "auto_retry_end"
            | "summarization_retry_scheduled"
            | "summarization_retry_attempt_start"
            | "summarization_retry_finished"
            | "extension_error" => Some(previous_len),
            _ => None,
        }
    }

    pub(crate) fn push_transport_error(&mut self, message: String) {
        self.running = false;
        self.items.push(Arc::new(TranscriptItem {
            kind: TranscriptKind::Error,
            label: "Connection error".into(),
            text: message,
            stream_chunks: Arc::default(),
            streaming: false,
            is_error: true,
            tool_call_id: None,
            tool_output: String::new(),
            tool_presentation: None,
        }));
    }

    pub(crate) fn push_extension_error(&mut self, message: String) {
        self.items.push(Arc::new(TranscriptItem {
            kind: TranscriptKind::Error,
            label: "Extension error".into(),
            text: message,
            stream_chunks: Arc::default(),
            streaming: false,
            is_error: true,
            tool_call_id: None,
            tool_output: String::new(),
            tool_presentation: None,
        }));
    }

    pub(crate) fn push_local_error(&mut self, label: &str, message: String) {
        self.items.push(Arc::new(TranscriptItem {
            kind: TranscriptKind::Error,
            label: label.into(),
            text: message,
            stream_chunks: Arc::default(),
            streaming: false,
            is_error: true,
            tool_call_id: None,
            tool_output: String::new(),
            tool_presentation: None,
        }));
    }

    fn start_message(&mut self, message: Option<&Value>) {
        self.content.clear();
        self.dirty_content.clear();
        self.projected_content.clear();
        if message.is_some_and(|message| {
            message.get("role").and_then(Value::as_str) == Some("toolResult")
                && message
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| {
                        self.items.iter().any(|item| {
                            item.kind == TranscriptKind::Tool
                                && item.tool_call_id.as_deref() == Some(id)
                        })
                    })
        }) {
            self.live_message = Some(LiveMessage {
                start: self.items.len(),
                len: 0,
            });
            return;
        }
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
            self.items[start] = Arc::new(projected.remove(0));
            self.live_message = Some(LiveMessage { start, len: 1 });
            return;
        }
        let start = self.items.len();
        let len = projected.len();
        self.items.extend(projected.into_iter().map(Arc::new));
        self.live_message = Some(LiveMessage { start, len });
    }

    fn update_message(&mut self, delta: Option<&Value>, project_live: bool) {
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
        self.dirty_content.insert(content_index);
        if project_live {
            self.flush_live_projection();
        }
    }

    pub(crate) fn flush_live_projection(&mut self) {
        let dirty = std::mem::take(&mut self.dirty_content);
        for content_index in dirty {
            let projection_existed = self.projected_content.contains(&content_index);
            if self.projected_content.is_empty() {
                self.clear_initial_live_projection();
            }
            self.refresh_live_projection(content_index, projection_existed);
            self.projected_content.insert(content_index);
        }
    }

    fn clear_initial_live_projection(&mut self) {
        let Some(mut live) = self.live_message else {
            return;
        };
        self.items.splice(live.start..live.start + live.len, []);
        live.len = 0;
        self.live_message = Some(live);
    }

    fn refresh_live_projection(&mut self, content_index: usize, content_existed: bool) {
        let Some(mut live) = self.live_message else {
            return;
        };
        let Some(partial) = self.content.get(&content_index) else {
            return;
        };
        let position = self.content.range(..content_index).count();
        let projected = Arc::new(TranscriptItem {
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
            stream_chunks: partial.chunks.clone(),
            streaming: true,
            is_error: false,
            tool_call_id: None,
            tool_output: String::new(),
            tool_presentation: None,
        });
        if content_existed {
            self.items[live.start + position] = projected;
        } else {
            self.items.insert(live.start + position, projected);
            live.len += 1;
            self.live_message = Some(live);
        }
    }

    fn end_message(&mut self, message: Option<&Value>) {
        let Some(message) = message else { return };
        if message.get("role").and_then(Value::as_str) == Some("assistant") {
            self.record_cache_hit_rate(message);
        }
        if message.get("role").and_then(Value::as_str) == Some("toolResult") {
            if let Some(live) = self.live_message.take() {
                self.items.splice(live.start..live.start + live.len, []);
            }
            let id = message
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if let Some(item) = self.items.iter_mut().rev().find(|item| {
                item.kind == TranscriptKind::Tool && item.tool_call_id.as_deref() == Some(id)
            }) {
                apply_tool_result(Arc::make_mut(item), message, true);
                self.content.clear();
                self.dirty_content.clear();
                self.projected_content.clear();
                return;
            }
        }
        let final_items = project_message_items(message)
            .into_iter()
            .map(Arc::new)
            .collect::<Vec<_>>();
        if let Some(live) = self.live_message.take() {
            self.items
                .splice(live.start..live.start + live.len, final_items);
        } else {
            self.items.extend(final_items);
        }
        self.content.clear();
        self.dirty_content.clear();
        self.projected_content.clear();
    }

    fn record_cache_hit_rate(&mut self, message: &Value) {
        let Some(rate) = cache_hit_rate(message) else {
            return;
        };
        self.cache_hit_rate_sum += rate;
        self.cache_hit_rate_count += 1;
        self.average_cache_hit_rate =
            Some(self.cache_hit_rate_sum / self.cache_hit_rate_count as f64);
    }

    fn start_tool(&mut self, event: &Value) {
        let id = text_field(event, "toolCallId");
        let name = text_field(event, "toolName");
        let args_value = event.get("args");
        let presentation = args_value.and_then(|args| tool_presentation(&name, args));
        let args = args_value.map(readable_json).unwrap_or_default();
        if let Some(index) = self.items.iter().rposition(|item| {
            item.kind == TranscriptKind::Tool && item.tool_call_id.as_deref() == Some(id.as_str())
        }) {
            if let Some(item) = self.items.get_mut(index).map(Arc::make_mut) {
                item.label = display_tool_name(&name);
                item.text = args;
                item.tool_presentation = presentation;
                item.streaming = true;
            }
            self.tools.insert(id, index);
            return;
        }
        self.items.push(Arc::new(TranscriptItem {
            kind: TranscriptKind::Tool,
            label: display_tool_name(&name),
            text: args,
            stream_chunks: Arc::default(),
            streaming: true,
            is_error: false,
            tool_call_id: Some(id.clone()),
            tool_output: String::new(),
            tool_presentation: presentation,
        }));
        self.tools.insert(id, self.items.len() - 1);
    }

    fn update_tool(&mut self, event: &Value) {
        let id = text_field(event, "toolCallId");
        if let Some(index) = self.tools.get(&id).copied()
            && let Some(item) = self.items.get_mut(index).map(Arc::make_mut)
        {
            item.tool_output = event
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
            && let Some(item) = self.items.get_mut(index).map(Arc::make_mut)
        {
            if let Some(result) = event.get("result") {
                apply_tool_result(item, result, false);
            }
            item.streaming = false;
            item.is_error = is_error;
        }
    }

    fn notice(&mut self, text: String) {
        self.items.push(Arc::new(TranscriptItem {
            kind: TranscriptKind::Notice,
            label: "Run".into(),
            text,
            stream_chunks: Arc::default(),
            streaming: false,
            is_error: false,
            tool_call_id: None,
            tool_output: String::new(),
            tool_presentation: None,
        }));
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
    partial.chunks = Arc::default();
}

fn append_delta(partial: &mut PartialContent, delta: &Value) {
    partial.value.push_str(
        delta
            .get("delta")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    if matches!(partial.kind, PartialKind::Text | PartialKind::Thinking) {
        freeze_stream_chunks(partial);
    }
}

fn freeze_stream_chunks(partial: &mut PartialContent) {
    while partial.value.len() > STREAM_TAIL_MAX_BYTES {
        let mut split = STREAM_CHUNK_BYTES.min(partial.value.len());
        while !partial.value.is_char_boundary(split) {
            split -= 1;
        }
        if let Some(newline) = partial.value[..split].rfind('\n')
            && newline > STREAM_CHUNK_BYTES / 2
        {
            split = newline + 1;
        }
        let chunk = partial.value[..split].to_owned();
        partial.value.drain(..split);
        Arc::make_mut(&mut partial.chunks).push(Arc::from(chunk));
    }
}

fn finish_content(partial: &mut PartialContent, delta: &Value) {
    if let Some(content) = delta.get("content").and_then(Value::as_str) {
        partial.value = content.to_owned();
        partial.chunks = Arc::default();
    }
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
        let mut items: Vec<TranscriptItem> = message
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
                            stream_chunks: Arc::default(),
                            streaming: false,
                            is_error,
                            tool_call_id: block
                                .get("id")
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                            tool_output: String::new(),
                            tool_presentation: tool_name(block).and_then(|name| {
                                block
                                    .get("arguments")
                                    .and_then(|arguments| tool_presentation(name, arguments))
                            }),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        if items.is_empty()
            && is_error
            && let Some(error) = message
                .get("errorMessage")
                .and_then(Value::as_str)
                .filter(|error| !error.is_empty())
        {
            items.push(TranscriptItem {
                kind: TranscriptKind::Error,
                label: "Model error".into(),
                text: error.to_owned(),
                stream_chunks: Arc::default(),
                streaming: false,
                is_error: true,
                tool_call_id: None,
                tool_output: String::new(),
                tool_presentation: None,
            });
        }
        return items;
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
        stream_chunks: Arc::default(),
        streaming: false,
        is_error,
        tool_call_id: message
            .get("toolCallId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        tool_output: String::new(),
        tool_presentation: None,
    }]
}

fn message_text(message: &Value) -> String {
    if let Some(content) = message.get("content")
        && let Some(text) = content.as_str()
    {
        return text.to_owned();
    }
    if let Some(blocks) = message.get("content").and_then(Value::as_array) {
        let text = blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        let image_count = blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("image"))
            .count();
        if !text.is_empty() || image_count > 0 {
            return user_message_text(&text, image_count);
        }
    }
    message
        .get("summary")
        .and_then(Value::as_str)
        .or_else(|| message.get("output").and_then(Value::as_str))
        .unwrap_or_default()
        .to_owned()
}

fn user_message_text(message: &str, image_count: usize) -> String {
    if image_count == 0 {
        return message.to_owned();
    }
    let attachment = if image_count == 1 {
        "Attached image".to_owned()
    } else {
        format!("Attached {image_count} images")
    };
    if message.is_empty() {
        attachment
    } else {
        format!("{message}\n\n{attachment}")
    }
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

fn tool_presentation(name: &str, arguments: &Value) -> Option<ToolPresentation> {
    let path = arguments
        .get("path")
        .or_else(|| arguments.get("file_path"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    match name.trim().to_ascii_lowercase().as_str() {
        "edit" => Some(ToolPresentation::Edit {
            path,
            diff: preview_edit_diff(arguments),
            format: EditDiffFormat::Unnumbered,
            prepared: Arc::default(),
        }),
        "write" => arguments
            .get("content")
            .and_then(Value::as_str)
            .map(|content| ToolPresentation::Write {
                path,
                content: content.to_owned(),
                prepared: Arc::default(),
            }),
        _ => None,
    }
}

fn preview_edit_diff(arguments: &Value) -> Option<String> {
    let edits = arguments.get("edits").and_then(Value::as_array);
    let legacy = arguments
        .get("oldText")
        .and_then(Value::as_str)
        .zip(arguments.get("newText").and_then(Value::as_str));
    let pairs = edits
        .map(|edits| {
            edits
                .iter()
                .filter_map(|edit| {
                    edit.get("oldText")
                        .and_then(Value::as_str)
                        .zip(edit.get("newText").and_then(Value::as_str))
                })
                .collect::<Vec<_>>()
        })
        .or_else(|| legacy.map(|pair| vec![pair]))?;
    let mut diff = String::new();
    for (index, (old, new)) in pairs.into_iter().enumerate() {
        if index > 0 {
            diff.push_str("     ...\n");
        }
        for line in old.lines() {
            diff.push_str("- ");
            diff.push_str(line);
            diff.push('\n');
        }
        for line in new.lines() {
            diff.push_str("+ ");
            diff.push_str(line);
            diff.push('\n');
        }
    }
    (!diff.is_empty()).then(|| diff.trim_end().to_owned())
}

fn apply_tool_result(item: &mut TranscriptItem, result: &Value, message: bool) {
    item.tool_output = if message {
        message_text(result)
    } else {
        result_text(result)
    };
    item.is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    item.streaming = false;
    if let Some(diff) = result
        .get("details")
        .and_then(|details| details.get("diff"))
        .and_then(Value::as_str)
        .filter(|diff| !diff.is_empty())
        && let Some(ToolPresentation::Edit {
            diff: item_diff,
            format,
            prepared,
            ..
        }) = item.tool_presentation.as_mut()
    {
        *item_diff = Some(diff.to_owned());
        *format = EditDiffFormat::Numbered;
        *prepared = Arc::default();
    }
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
