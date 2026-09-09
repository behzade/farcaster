use std::collections::{HashMap, HashSet, VecDeque};

use crate::agents::{
    TokenUsage, ToolCategory, ToolMetadata, WorkerActivity, WorkerEvent, WorkerUsage,
};
use serde_json::{Value, json};

pub(super) fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

pub(super) fn blocks(message: &Value) -> Vec<Value> {
    match &message["content"] {
        Value::String(text) => vec![json!({"type":"text","text":text})],
        Value::Array(blocks) => blocks.clone(),
        _ => Vec::new(),
    }
}

pub(super) fn text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        _ => content.to_string(),
    }
}

pub(super) fn tool_metadata(name: &str, input: &Value) -> ToolMetadata {
    let category = match name {
        "Read" => ToolCategory::Read,
        "Grep" => ToolCategory::Search,
        "Glob" => ToolCategory::List,
        "Edit" | "Write" | "NotebookEdit" => ToolCategory::Change,
        "Bash" => ToolCategory::Execute,
        "WebFetch" | "WebSearch" => ToolCategory::Fetch,
        "Agent" | "Task" => ToolCategory::Delegate,
        _ => ToolCategory::Other,
    };
    ToolMetadata {
        category: Some(category),
        targets: ["file_path", "path", "notebook_path"]
            .iter()
            .filter_map(|key| input[*key].as_str().map(str::to_owned))
            .collect(),
        ..Default::default()
    }
}

pub(super) fn tokens(usage: &Value) -> TokenUsage {
    TokenUsage {
        input: usage["input_tokens"].as_u64().unwrap_or(0),
        output: usage["output_tokens"].as_u64().unwrap_or(0),
        cache_read: usage["cache_read_input_tokens"].as_u64().unwrap_or(0),
        cache_write: usage["cache_creation_input_tokens"].as_u64().unwrap_or(0),
    }
}

/// Project both live and saved API content into Farcaster's neutral transcript.
pub(super) fn history_messages(message: &Value) -> Vec<Value> {
    let mut messages = Vec::new();
    let mut content = Vec::new();
    for block in blocks(message) {
        match string(&block, "type") {
            "tool_result" => messages.push(json!({"role":"toolResult",
                "toolCallId":block["tool_use_id"], "isError":block["is_error"].as_bool().unwrap_or(false),
                "content":[{"type":"text","text":text(&block["content"])}]})),
            "tool_use" => content.push(json!({"type":"toolCall", "id":block["id"],
                "name":block["name"], "arguments":block["input"],
                "metadata":tool_metadata(string(&block,"name"), &block["input"])})),
            "thinking" => content.push(json!({"type":"thinking", "thinking":block["thinking"]})),
            "text" => content.push(block),
            "image" => content.push(json!({"type":"image", "mimeType":block["source"]["media_type"],
                "data":block["source"]["data"]})),
            _ => {},
        }
    }
    if !content.is_empty() {
        messages.push(json!({"role":message["role"], "content":content}));
    }
    messages
}

#[derive(Default)]
pub(super) struct Events {
    pub(super) pending: VecDeque<WorkerEvent>,
    pub(super) output: String,
    streamed: HashMap<usize, String>,
    thinking: HashMap<usize, String>,
    tools: HashSet<String>,
    session_usage: TokenUsage,
    context_usage: TokenUsage,
    compacting: bool,
}

impl Events {
    pub(super) fn activity(&mut self, activity: WorkerActivity) {
        self.pending.push_back(WorkerEvent::Activity(activity));
    }

    pub(super) fn start(&mut self) {
        self.output.clear();
        self.streamed.clear();
        self.thinking.clear();
        self.tools.clear();
        self.pending.push_back(WorkerEvent::Started);
    }

    fn delta(&mut self, index: usize, delta: &str, thinking: bool) {
        if delta.is_empty() {
            return;
        }
        if thinking {
            if !self.thinking.contains_key(&index) {
                self.activity(WorkerActivity::ThinkingStarted {
                    content_index: index,
                });
            }
            self.thinking.entry(index).or_default().push_str(delta);
            self.activity(WorkerActivity::ThinkingDelta {
                content_index: index,
                delta: delta.into(),
            });
        } else {
            self.streamed.entry(index).or_default().push_str(delta);
            self.output.push_str(delta);
            self.activity(WorkerActivity::TextDelta {
                content_index: index,
                delta: delta.into(),
            });
        }
    }

    /// Input has already passed the source-derived SDK envelope decoder.
    pub(super) fn message(&mut self, frame: &Value) {
        // Native subagent streams belong to their parent tool, not the main answer.
        if frame["parent_tool_use_id"].as_str().is_some() {
            return;
        }
        match string(frame, "type") {
            "stream_event" => {
                let event = &frame["event"];
                let index = event["index"].as_u64().unwrap_or(0) as usize;
                match string(event, "type") {
                    "message_start" => {
                        self.streamed.clear(); self.thinking.clear();
                        self.context_usage = tokens(&event["message"]["usage"]);
                    }
                    "content_block_start" => {
                        let block = &event["content_block"];
                        match string(block,"type") {
                            "text" => self.delta(index, string(block,"text"), false),
                            "thinking" => self.delta(index, string(block,"thinking"), true),
                            _ => {},
                        }
                    }
                    "content_block_delta" => {
                        let delta = &event["delta"];
                        match string(delta,"type") {
                            "text_delta" => self.delta(index, string(delta,"text"), false),
                            "thinking_delta" => self.delta(index, string(delta,"thinking"), true),
                            _ => {},
                        }
                    }
                    _ => {},
                }
            }
            "assistant" => {
                let message = &frame["message"];
                self.context_usage = tokens(&message["usage"]);
                for (index, block) in blocks(message).iter().enumerate() {
                    match string(block,"type") {
                        "text" | "thinking" => {
                            let thinking = block["type"] == "thinking";
                            let full = string(block, if thinking {"thinking"} else {"text"});
                            let previous = if thinking {&self.thinking} else {&self.streamed}
                                .get(&index).map(String::as_str).unwrap_or_default();
                            // The complete message repeats streamed content; emit only its suffix.
                            if let Some(suffix) = full.strip_prefix(previous) {
                                self.delta(index, suffix, thinking);
                            }
                        }
                        "tool_use" => {
                            let id = string(block,"id");
                            if self.tools.insert(id.into()) {
                                self.activity(WorkerActivity::ToolStarted { id:id.into(),
                                    name:string(block,"name").into(), args:block["input"].clone(),
                                    metadata:tool_metadata(string(block,"name"), &block["input"]) });
                            }
                        }
                        _ => {},
                    }
                }
                self.streamed.clear(); self.thinking.clear();
            }
            "user" => {
                for block in blocks(&frame["message"]) {
                    if block["type"] == "tool_result" && self.tools.remove(string(&block,"tool_use_id")) {
                        self.activity(WorkerActivity::ToolFinished { id:string(&block,"tool_use_id").into(),
                            result:json!({"content":[{"type":"text","text":text(&block["content"])}]}),
                            is_error:block["is_error"].as_bool().unwrap_or(false) });
                    }
                }
            }
            "result" => {
                // result.usage is the query total, not the last API request's context size.
                self.session_usage = self.session_usage.saturating_add(tokens(&frame["usage"]));
                let context_window = frame["modelUsage"].as_object().into_iter().flat_map(|models| models.values())
                    .filter_map(|usage| usage["contextWindow"].as_u64()).max().unwrap_or(0);
                self.activity(WorkerActivity::Usage(WorkerUsage { turn:self.context_usage,
                    session:self.session_usage, context_window }));
            }
            "tool_progress" => self.activity(WorkerActivity::ToolUpdated {
                id:string(frame,"tool_use_id").into(),
                content:json!([{"type":"text","text":format!("Running for {}s", frame["elapsed_time_seconds"])}]) }),
            "rate_limit_event" => self.activity(WorkerActivity::RateLimitsChanged { limits:frame["rate_limit_info"].clone() }),
            "system" => match string(frame,"subtype") {
                "init" => {
                    self.activity(WorkerActivity::ModeChanged(string(frame,"permissionMode").into()));
                    if let Some(servers) = frame["mcp_servers"].as_array() {
                        for server in servers {
                            self.activity(WorkerActivity::ServiceStatusChanged { name:string(server,"name").into(),
                                status:string(server,"status").into(), error:None, failure_reason:None });
                        }
                    }
                }
                "status" => {
                    if let Some(mode) = frame["permissionMode"].as_str() {
                        self.activity(WorkerActivity::ModeChanged(mode.into()));
                    }
                    if frame["status"] == "compacting" && !self.compacting {
                        self.compacting = true;
                        self.activity(WorkerActivity::CompactionStarted);
                    } else if frame["status"].is_null() && self.compacting {
                        self.compacting = false;
                        self.activity(WorkerActivity::CompactionFinished { aborted:false, error:frame["compact_error"].as_str().map(str::to_owned) });
                    }
                }
                "compact_boundary" if self.compacting => {
                    self.compacting = false;
                    self.activity(WorkerActivity::CompactionFinished { aborted:false, error:None });
                }
                "local_command_output" => self.delta(0,string(frame,"content"),false),
                "commands_changed" => {
                    let commands = frame["commands"].as_array().into_iter().flatten().map(|command|json!({
                        "name":command["name"],"description":command["description"],"source":"prompt",
                    })).collect();
                    self.activity(WorkerActivity::CommandsChanged { commands });
                }
                _ => {},
            },
            _ => {},
        }
    }
}
