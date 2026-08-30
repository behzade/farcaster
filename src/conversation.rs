use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use base64::Engine as _;
use gpui::{Image, ImageFormat};
use serde_json::Value;

use crate::{persistent_vec::PersistentVec, protocol::PromptImage};

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
    AgentResult,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolPresentation {
    path: String,
    additions: usize,
    deletions: usize,
    first_changed_line: Option<u64>,
}

impl ToolPresentation {
    fn edit(path: String, (additions, deletions): (usize, usize)) -> Self {
        Self {
            path,
            additions,
            deletions,
            first_changed_line: None,
        }
    }

    fn write(path: String, content: &str) -> Self {
        Self {
            path,
            additions: content.lines().count(),
            deletions: 0,
            first_changed_line: None,
        }
    }

    fn apply_edit_result(&mut self, diff: &str, first_changed_line: Option<u64>) {
        (self.additions, self.deletions) = change_counts(diff);
        self.first_changed_line = first_changed_line;
    }

    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    pub(crate) const fn counts(&self) -> (usize, usize) {
        (self.additions, self.deletions)
    }

    pub(crate) const fn first_changed_line(&self) -> Option<u64> {
        self.first_changed_line
    }
}

fn change_counts(diff: &str) -> (usize, usize) {
    diff.lines().fold((0, 0), |(additions, deletions), line| {
        match line.as_bytes().first() {
            Some(b'+') => (additions + 1, deletions),
            Some(b'-') => (additions, deletions + 1),
            _ => (additions, deletions),
        }
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TranscriptItem {
    pub kind: TranscriptKind,
    pub label: String,
    pub text: String,
    pub images: Arc<Vec<Arc<Image>>>,
    pub stream_chunks: Arc<Vec<Arc<str>>>,
    pub streaming: bool,
    pub is_error: bool,
    pub tool_call_id: Option<String>,
    pub tool_output: String,
    pub tool_presentation: Option<ToolPresentation>,
    pub invocation: Option<String>,
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

impl QueueState {
    fn acknowledge(&mut self, message: &str) {
        for queue in [&mut self.steering, &mut self.follow_up] {
            if let Some(index) = queue.iter().position(|queued| queued == message) {
                queue.remove(index);
                return;
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ConversationState {
    pub items: PersistentVec<Arc<TranscriptItem>>,
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
    optimistic_user: Option<Arc<TranscriptItem>>,
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
        invocation: bool,
    ) -> Arc<TranscriptItem> {
        self.push_local_user_with_images(
            user_message_text(&message, image_count),
            Arc::default(),
            invocation,
        )
    }

    pub(crate) fn push_local_user_with_prompt_images(
        &mut self,
        message: String,
        images: &[PromptImage],
        invocation: bool,
    ) -> Arc<TranscriptItem> {
        self.push_local_user_with_images(message, decode_prompt_images(images), invocation)
    }

    fn push_local_user_with_images(
        &mut self,
        message: String,
        images: Arc<Vec<Arc<Image>>>,
        invocation: bool,
    ) -> Arc<TranscriptItem> {
        let item = Arc::new(TranscriptItem {
            kind: TranscriptKind::User,
            label: String::new(),
            text: message,
            images,
            stream_chunks: Arc::default(),
            streaming: false,
            is_error: false,
            tool_call_id: None,
            tool_output: String::new(),
            tool_presentation: None,
            invocation: invocation.then(String::new),
        });
        self.items.push(item.clone());
        self.optimistic_user = Some(item.clone());
        item
    }

    pub(crate) fn rollback_local_user(&mut self, optimistic: &Arc<TranscriptItem>) -> bool {
        let Some(index) = self.items.rposition(|item| Arc::ptr_eq(item, optimistic)) else {
            return false;
        };
        self.items.remove(index);
        if self
            .optimistic_user
            .as_ref()
            .is_some_and(|item| Arc::ptr_eq(item, optimistic))
        {
            self.optimistic_user = None;
        }
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
        self.optimistic_user = None;
    }

    fn project_history_message(&mut self, message: &Value) {
        if message.get("role").and_then(Value::as_str) == Some("toolResult") {
            let id = message
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !id.is_empty()
                && let Some(index) = self.items.rposition(|item| {
                    item.kind == TranscriptKind::Tool && item.tool_call_id.as_deref() == Some(id)
                })
            {
                let mut item = self.items[index].clone();
                apply_tool_result(Arc::make_mut(&mut item), message, true);
                self.items.set(index, item);
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

    #[allow(dead_code)] // Used by the standalone transcript benchmark.
    pub(crate) fn reduce(&mut self, event: &Value) -> Option<usize> {
        self.reduce_with_projection(event, true)
    }

    pub(crate) fn reduce_deferred(&mut self, event: &Value) -> Option<usize> {
        self.reduce_with_projection(event, false)
    }

    pub(crate) fn reduce_deferred_with_change(&mut self, event: &Value) -> (Option<usize>, bool) {
        let previous_state = (
            self.running,
            self.settled,
            self.compacting,
            self.retrying,
            self.queue.clone(),
            self.average_cache_hit_rate,
            self.diagnostics.len(),
        );
        let changed_from = self.reduce_deferred(event);
        let state_changed = previous_state
            != (
                self.running,
                self.settled,
                self.compacting,
                self.retrying,
                self.queue.clone(),
                self.average_cache_hit_rate,
                self.diagnostics.len(),
            );
        (changed_from, state_changed)
    }

    fn reduce_with_projection(&mut self, event: &Value, project_live: bool) -> Option<usize> {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let previous_len = self.items.len();
        let previous_live_start = self.live_message.map(|live| live.start);
        let mut incremental_content_changed = true;
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
                incremental_content_changed =
                    self.update_message(event.get("assistantMessageEvent"), project_live);
            }
            "message_end" => self.end_message(event.get("message")),
            "tool_execution_start" => self.start_tool(event),
            "tool_execution_update" => incremental_content_changed = self.update_tool(event),
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
            "message_update" if incremental_content_changed => previous_live_start,
            "message_update" => None,
            "message_end" => previous_live_start,
            "tool_execution_update" if !incremental_content_changed => None,
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
            images: Arc::default(),
            stream_chunks: Arc::default(),
            streaming: false,
            is_error: true,
            tool_call_id: None,
            tool_output: String::new(),
            tool_presentation: None,
            invocation: None,
        }));
    }

    pub(crate) fn push_extension_error(&mut self, message: String) {
        self.items.push(Arc::new(TranscriptItem {
            kind: TranscriptKind::Error,
            label: "Extension error".into(),
            text: message,
            images: Arc::default(),
            stream_chunks: Arc::default(),
            streaming: false,
            is_error: true,
            tool_call_id: None,
            tool_output: String::new(),
            tool_presentation: None,
            invocation: None,
        }));
    }

    pub(crate) fn push_local_error(&mut self, label: &str, message: String) {
        self.push_local_error_with_details(label, message, String::new());
    }

    pub(crate) fn push_local_error_with_details(
        &mut self,
        label: &str,
        message: String,
        details: String,
    ) {
        self.items.push(Arc::new(TranscriptItem {
            kind: TranscriptKind::Error,
            label: label.into(),
            text: message,
            images: Arc::default(),
            stream_chunks: Arc::default(),
            streaming: false,
            is_error: true,
            tool_call_id: None,
            tool_output: details,
            tool_presentation: None,
            invocation: None,
        }));
    }

    fn start_message(&mut self, message: Option<&Value>) {
        self.content.clear();
        self.dirty_content.clear();
        self.projected_content.clear();
        if let Some(message) = message
            && message.get("role").and_then(Value::as_str) == Some("user")
        {
            self.queue.acknowledge(&message_text(message));
        }
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
        if projected.len() == 1
            && projected[0].kind == TranscriptKind::User
            && self.optimistic_user.is_some()
        {
            self.live_message = Some(LiveMessage {
                start: self.items.len(),
                len: 0,
            });
            return;
        }
        for item in &mut projected {
            item.streaming = true;
        }
        let start = self.items.len();
        let len = projected.len();
        self.items.extend(projected.into_iter().map(Arc::new));
        self.live_message = Some(LiveMessage { start, len });
    }

    fn update_message(&mut self, delta: Option<&Value>, project_live: bool) -> bool {
        let Some(delta) = delta else { return false };
        let Some(content_index) = delta
            .get("contentIndex")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
        else {
            return false;
        };
        let delta_type = delta
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let partial = self.content.entry(content_index).or_default();
        let changed = match delta_type {
            "text_start" => reset_partial(partial, PartialKind::Text),
            "text_delta" => append_delta(partial, delta),
            "text_end" => finish_content(partial, delta),
            "thinking_start" => reset_partial(partial, PartialKind::Thinking),
            "thinking_delta" => append_delta(partial, delta),
            "thinking_end" => finish_content(partial, delta),
            "toolcall_start" => {
                let label = delta
                    .get("toolCall")
                    .and_then(tool_name)
                    .or_else(|| delta.get("toolName").and_then(Value::as_str))
                    .map(display_tool_name)
                    .unwrap_or_default();
                let changed =
                    reset_partial(partial, PartialKind::ToolCall) || partial.label != label;
                partial.label = label;
                changed
            }
            "toolcall_delta" => append_delta(partial, delta),
            "toolcall_end" => delta.get("toolCall").is_some_and(|tool_call| {
                let label = tool_name(tool_call)
                    .map(display_tool_name)
                    .unwrap_or_else(|| "Tool".into());
                let value = tool_arguments(tool_call);
                let changed = partial.label != label || partial.value != value;
                partial.label = label;
                partial.value = value;
                changed
            }),
            _ => return false,
        };
        if !changed {
            return false;
        }
        self.dirty_content.insert(content_index);
        if project_live {
            self.flush_live_projection();
        }
        true
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
            images: Arc::default(),
            stream_chunks: partial.chunks.clone(),
            streaming: true,
            is_error: false,
            tool_call_id: None,
            tool_output: String::new(),
            tool_presentation: None,
            invocation: None,
        });
        if content_existed {
            self.items.set(live.start + position, projected);
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
            if let Some(index) = self.items.rposition(|item| {
                item.kind == TranscriptKind::Tool && item.tool_call_id.as_deref() == Some(id)
            }) {
                let mut item = self.items[index].clone();
                apply_tool_result(Arc::make_mut(&mut item), message, true);
                self.items.set(index, item);
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
        let finalizes_user = final_items
            .iter()
            .any(|item| item.kind == TranscriptKind::User);
        if let Some(live) = self.live_message.take() {
            self.items
                .splice(live.start..live.start + live.len, final_items);
        } else {
            self.items.extend(final_items);
        }
        if finalizes_user
            && let Some(optimistic) = self.optimistic_user.take()
            && let Some(index) = self.items.position(|item| Arc::ptr_eq(item, &optimistic))
        {
            self.items.remove(index);
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
        let args = args_value
            .map(|args| format_tool_arguments(&name, args))
            .unwrap_or_default();
        if let Some(index) = self.items.rposition(|item| {
            item.kind == TranscriptKind::Tool && item.tool_call_id.as_deref() == Some(id.as_str())
        }) {
            let mut item = self.items[index].clone();
            let value = Arc::make_mut(&mut item);
            value.label = display_tool_name(&name);
            value.text = args;
            value.tool_presentation = presentation;
            value.streaming = true;
            self.items.set(index, item);
            self.tools.insert(id, index);
            return;
        }
        self.items.push(Arc::new(TranscriptItem {
            kind: TranscriptKind::Tool,
            label: display_tool_name(&name),
            text: args,
            images: Arc::default(),
            stream_chunks: Arc::default(),
            streaming: true,
            is_error: false,
            tool_call_id: Some(id.clone()),
            tool_output: String::new(),
            tool_presentation: presentation,
            invocation: None,
        }));
        self.tools.insert(id, self.items.len() - 1);
    }

    fn update_tool(&mut self, event: &Value) -> bool {
        let id = text_field(event, "toolCallId");
        let Some(index) = self.tools.get(&id).copied() else {
            return false;
        };
        let output = event
            .get("partialResult")
            .map(result_text)
            .unwrap_or_default();
        let Some(item) = self.items.get(index) else {
            return false;
        };
        if item.tool_output == output {
            return false;
        }
        let mut item = item.clone();
        Arc::make_mut(&mut item).tool_output = output;
        self.items.set(index, item);
        true
    }

    fn end_tool(&mut self, event: &Value) {
        let id = text_field(event, "toolCallId");
        let is_error = event
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(index) = self.tools.remove(&id)
            && let Some(item) = self.items.get(index)
        {
            let mut item = item.clone();
            let value = Arc::make_mut(&mut item);
            if let Some(result) = event.get("result") {
                apply_tool_result(value, result, false);
            }
            value.streaming = false;
            value.is_error = is_error;
            self.items.set(index, item);
        }
    }

    fn notice(&mut self, text: String) {
        self.items.push(Arc::new(TranscriptItem {
            kind: TranscriptKind::Notice,
            label: "Run".into(),
            text,
            images: Arc::default(),
            stream_chunks: Arc::default(),
            streaming: false,
            is_error: false,
            tool_call_id: None,
            tool_output: String::new(),
            tool_presentation: None,
            invocation: None,
        }));
    }

    fn diagnostic(&mut self, text: String) {
        self.diagnostics.push(text);
        if self.diagnostics.len() > MAX_DIAGNOSTICS {
            self.diagnostics.remove(0);
        }
    }
}

fn reset_partial(partial: &mut PartialContent, kind: PartialKind) -> bool {
    let changed = partial.kind != kind
        || !partial.label.is_empty()
        || !partial.value.is_empty()
        || !partial.chunks.is_empty();
    partial.kind = kind;
    partial.label.clear();
    partial.value.clear();
    partial.chunks = Arc::default();
    changed
}

fn append_delta(partial: &mut PartialContent, delta: &Value) -> bool {
    let text = delta
        .get("delta")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let freezes_tail = matches!(partial.kind, PartialKind::Text | PartialKind::Thinking)
        && partial.value.len() > STREAM_TAIL_MAX_BYTES;
    let changed = !text.is_empty() || freezes_tail;
    partial.value.push_str(text);
    if matches!(partial.kind, PartialKind::Text | PartialKind::Thinking) {
        freeze_stream_chunks(partial);
    }
    changed
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

fn finish_content(partial: &mut PartialContent, delta: &Value) -> bool {
    let Some(content) = delta.get("content").and_then(Value::as_str) else {
        return false;
    };
    let changed = partial.value != content || !partial.chunks.is_empty();
    if changed {
        partial.value = content.to_owned();
        partial.chunks = Arc::default();
    }
    changed
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
                            Some("thinking") => {
                                let thinking = block
                                    .get("thinking")
                                    .and_then(Value::as_str)
                                    .filter(|thinking| !thinking.trim().is_empty())?;
                                (TranscriptKind::Thinking, String::new(), thinking.to_owned())
                            }
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
                            kind,
                            label,
                            text,
                            images: Arc::default(),
                            stream_chunks: Arc::default(),
                            streaming: false,
                            is_error: false,
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
                            invocation: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        if is_error && let Some(error) = assistant_error_text(message, !items.is_empty()) {
            items.push(model_error_item(error));
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
            if message.get("customType").and_then(Value::as_str) == Some("subagent-result") {
                TranscriptKind::AgentResult
            } else {
                TranscriptKind::Custom
            },
            if message.get("customType").and_then(Value::as_str) == Some("subagent-result") {
                "Subagent result".into()
            } else {
                "Extension".into()
            },
            message.get("customType").and_then(Value::as_str) != Some("background-job-result")
                && message
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
    let invocation = if kind == TranscriptKind::User {
        message
            .get("piUserInvocation")
            .and_then(Value::as_str)
            .map(|_| message_text(message))
    } else {
        None
    };
    vec![TranscriptItem {
        kind: if is_error && kind != TranscriptKind::Tool {
            TranscriptKind::Error
        } else {
            kind
        },
        label,
        text: if kind == TranscriptKind::User {
            projected_user_message_text(message)
        } else {
            message_text(message)
        },
        images: if kind == TranscriptKind::User {
            message_images(message)
        } else {
            Arc::default()
        },
        stream_chunks: Arc::default(),
        streaming: false,
        is_error,
        tool_call_id: message
            .get("toolCallId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        tool_output: String::new(),
        tool_presentation: None,
        invocation,
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
        let has_image = blocks
            .iter()
            .any(|block| block.get("type").and_then(Value::as_str) == Some("image"));
        if !text.is_empty() || has_image {
            return text;
        }
    }
    message
        .get("summary")
        .and_then(Value::as_str)
        .or_else(|| message.get("output").and_then(Value::as_str))
        .unwrap_or_default()
        .to_owned()
}

fn projected_user_message_text(message: &Value) -> String {
    let text = message
        .get("piUserInvocation")
        .and_then(Value::as_str)
        .map_or_else(|| message_text(message), str::to_owned);
    pasted_file_summary(&text).to_owned()
}

fn user_message_text(message: &str, image_count: usize) -> String {
    let message = pasted_file_summary(message);
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

fn pasted_file_summary(message: &str) -> &str {
    message
        .split_once("\n\n--- BEGIN PASTED FILE ")
        .map_or(message, |(summary, _)| summary)
}

fn decode_prompt_images(images: &[PromptImage]) -> Arc<Vec<Arc<Image>>> {
    decode_images(
        images
            .iter()
            .map(|image| (image.data.as_str(), image.mime_type.as_str())),
    )
}

fn message_images(message: &Value) -> Arc<Vec<Arc<Image>>> {
    decode_images(
        message
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("image"))
            .filter_map(|block| {
                block
                    .get("data")
                    .and_then(Value::as_str)
                    .zip(block.get("mimeType").and_then(Value::as_str))
            }),
    )
}

fn decode_images<'a>(images: impl IntoIterator<Item = (&'a str, &'a str)>) -> Arc<Vec<Arc<Image>>> {
    Arc::new(
        images
            .into_iter()
            .filter_map(|(data, mime_type)| decode_image(data, mime_type))
            .collect(),
    )
}

fn decode_image(data: &str, mime_type: &str) -> Option<Arc<Image>> {
    let format = ImageFormat::from_mime_type(mime_type)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()?;
    (!bytes.is_empty()).then(|| Arc::new(Image::from_bytes(format, bytes)))
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
        .map_or_else(String::new, |arguments| {
            format_tool_arguments(tool_name(value).unwrap_or_default(), arguments)
        })
}

fn format_tool_arguments(name: &str, arguments: &Value) -> String {
    if name == "request_user_input"
        && let Some(script) = arguments.get("script").and_then(Value::as_str)
        && !script.is_empty()
    {
        let question = arguments
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        return if question.is_empty() {
            format!("{COMMAND_BLOCK_PREFIX}{script}")
        } else {
            format!("{question}\n\n{COMMAND_BLOCK_PREFIX}{script}")
        };
    }
    readable_json(arguments)
}

const COMMAND_BLOCK_MARKER: &str = "\n\nCommand:\n";
const COMMAND_BLOCK_PREFIX: &str = "Command:\n";

pub(crate) fn split_command_block(text: &str) -> Option<(&str, &str)> {
    if let Some(index) = text.rfind(COMMAND_BLOCK_MARKER) {
        return Some((&text[..index], &text[index + COMMAND_BLOCK_MARKER.len()..]));
    }
    text.strip_prefix(COMMAND_BLOCK_PREFIX)
        .map(|command| ("", command))
}

fn tool_presentation(name: &str, arguments: &Value) -> Option<ToolPresentation> {
    let path = arguments
        .get("path")
        .or_else(|| arguments.get("file_path"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    match name.trim().to_ascii_lowercase().as_str() {
        "edit" => Some(ToolPresentation::edit(path, preview_edit_counts(arguments))),
        "write" => arguments
            .get("content")
            .and_then(Value::as_str)
            .map(|content| ToolPresentation::write(path, content)),
        _ => None,
    }
}

fn preview_edit_counts(arguments: &Value) -> (usize, usize) {
    let edits = arguments.get("edits").and_then(Value::as_array);
    let legacy = arguments
        .get("oldText")
        .and_then(Value::as_str)
        .zip(arguments.get("newText").and_then(Value::as_str));
    if let Some(edits) = edits {
        return edit_pair_counts(edits.iter().filter_map(|edit| {
            edit.get("oldText")
                .and_then(Value::as_str)
                .zip(edit.get("newText").and_then(Value::as_str))
        }));
    }
    edit_pair_counts(legacy)
}

fn edit_pair_counts<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> (usize, usize) {
    pairs
        .into_iter()
        .fold((0, 0), |(additions, deletions), (old, new)| {
            (
                additions.saturating_add(new.lines().count()),
                deletions.saturating_add(old.lines().count()),
            )
        })
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
    let details = result.get("details");
    if let Some(diff) = details
        .and_then(|details| details.get("diff"))
        .and_then(Value::as_str)
        .filter(|diff| !diff.is_empty())
        && let Some(presentation) = item.tool_presentation.as_mut()
    {
        let first_changed_line = details
            .and_then(|details| details.get("firstChangedLine"))
            .and_then(Value::as_u64);
        presentation.apply_edit_result(diff, first_changed_line);
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
fn assistant_error_text(message: &Value, has_content: bool) -> Option<String> {
    message
        .get("errorMessage")
        .and_then(Value::as_str)
        .filter(|error| !error.is_empty())
        .map(str::to_owned)
        .or_else(|| has_content.then(|| "Unknown error".into()))
}

fn model_error_item(text: String) -> TranscriptItem {
    TranscriptItem {
        kind: TranscriptKind::Error,
        label: "Model error".into(),
        text,
        images: Arc::default(),
        stream_chunks: Arc::default(),
        streaming: false,
        is_error: true,
        tool_call_id: None,
        tool_output: String::new(),
        tool_presentation: None,
        invocation: None,
    }
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
