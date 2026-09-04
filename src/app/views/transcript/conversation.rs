use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use base64::Engine as _;
use gpui::{Image, ImageFormat};
use serde_json::Value;

use crate::{
    agents::{CommonTool, PeerMessage},
    app::ui::persistent_vec::PersistentVec,
    protocol::PromptImage,
};

#[path = "conversation/history.rs"]
mod history;
#[path = "conversation/stream.rs"]
mod stream;
#[path = "conversation/tools.rs"]
mod tools;

pub(crate) use history::annotate_prompt_presentations;
use history::{
    decode_prompt_images, message_text, pasted_file_summary, peer_transcript_item,
    project_message_items, user_message_text,
};
use tools::{apply_tool_result, tool_arguments, tool_name, tool_presentation};
pub(crate) use tools::{display_tool_name, split_command_block};

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
    PeerMessage,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolReviewState {
    Reviewing,
    Approved,
    Blocked,
}

impl ToolReviewState {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Reviewing => "in progress",
            Self::Approved => "approved",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolReview {
    pub state: ToolReviewState,
    pub detail: Option<String>,
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
    pub tool_review: Option<ToolReview>,
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
    cache_read_tokens: u64,
    prompt_tokens: u64,
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
            invocation.then(String::new),
        )
    }

    pub(crate) fn push_local_invocation(
        &mut self,
        message: String,
        image_count: usize,
        resolution: String,
    ) -> Arc<TranscriptItem> {
        self.push_local_user_with_images(
            user_message_text(&message, image_count),
            Arc::default(),
            Some(resolution),
        )
    }

    pub(crate) fn push_local_user_with_prompt_images(
        &mut self,
        message: String,
        images: &[PromptImage],
        invocation: bool,
    ) -> Arc<TranscriptItem> {
        self.push_local_user_with_images(
            pasted_file_summary(&message).to_owned(),
            decode_prompt_images(images),
            invocation.then(String::new),
        )
    }

    pub(crate) fn push_local_invocation_with_prompt_images(
        &mut self,
        message: String,
        images: &[PromptImage],
        resolution: String,
    ) -> Arc<TranscriptItem> {
        self.push_local_user_with_images(
            pasted_file_summary(&message).to_owned(),
            decode_prompt_images(images),
            Some(resolution),
        )
    }

    fn push_local_user_with_images(
        &mut self,
        message: String,
        images: Arc<Vec<Arc<Image>>>,
        invocation: Option<String>,
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
            tool_review: None,
            invocation,
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

    pub(crate) fn ended_in_error(&self) -> bool {
        self.items
            .iter_rev()
            .find(|item| item.kind != TranscriptKind::Notice)
            .is_some_and(|item| item.kind == TranscriptKind::Error)
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
            tool_review: None,
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
            tool_review: None,
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
            tool_review: None,
            invocation: None,
        }));
    }
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
#[cfg(test)]
#[path = "conversation/tests.rs"]
mod tests;
