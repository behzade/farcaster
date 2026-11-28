use std::{
    borrow::Cow,
    collections::{HashMap, HashSet, VecDeque},
};

use crate::adapter::stream_text::TextUpdate;
use crate::{TokenUsage, ToolCategory, ToolMetadata, WorkerActivity, WorkerEvent, WorkerUsage};
use serde_json::{Value, json};

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;

pub(super) fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

pub(super) fn blocks(message: &Value) -> Cow<'_, [Value]> {
    match &message["content"] {
        Value::String(text) => Cow::Owned(vec![json!({"type":"text","text":text})]),
        Value::Array(blocks) => Cow::Borrowed(blocks),
        _ => Cow::Borrowed(&[]),
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

fn tool_content(content: &Value) -> Vec<Value> {
    match content {
        Value::Array(blocks) => blocks
            .iter()
            .map(|block| match string(block, "type") {
                "image" if block["source"]["type"] == "base64" => json!({
                    "type":"image", "data":block["source"]["data"],
                    "mimeType":block["source"]["media_type"]
                }),
                _ => block.clone(),
            })
            .collect(),
        _ => vec![json!({"type":"text","text":text(content)})],
    }
}

fn native_tool_result<'a>(frame: &'a Value, blocks: &[Value]) -> Option<&'a Value> {
    let native = frame
        .get("tool_use_result")
        .or_else(|| frame.get("toolUseResult"))?;
    (!native.is_null()
        && blocks
            .iter()
            .filter(|block| block["type"] == "tool_result")
            .count()
            == 1)
        .then_some(native)
}

fn tool_result(block: &Value, native: Option<&Value>) -> Value {
    let mut result = json!({"content":tool_content(&block["content"])});
    if let Some(native) = native {
        result["details"] = json!({"native": native});
    }
    result
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

pub(super) fn history_messages(frame: &Value) -> Vec<Value> {
    let message = &frame["message"];
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        return vec![json!({"role":message["role"], "content":[{"type":"text", "text":text}]})];
    }
    let mut messages = Vec::new();
    let mut content = Vec::new();
    let blocks = blocks(message);
    let native = native_tool_result(frame, &blocks);
    for block in blocks.iter() {
        match string(block, "type") {
            "tool_result" => {
                let mut result = tool_result(block, native);
                result["role"] = "toolResult".into();
                result["toolCallId"] = block["tool_use_id"].clone();
                result["isError"] = block["is_error"].as_bool().unwrap_or(false).into();
                messages.push(result);
            }
            "tool_use" => content.push(json!({"type":"toolCall", "id":block["id"],
                "name":block["name"], "arguments":block["input"],
                "toolMetadata":tool_metadata(string(block,"name"), &block["input"])})),
            "thinking" => content.push(json!({"type":"thinking", "thinking":block["thinking"]})),
            "text" => content.push(block.clone()),
            "image" => content.push(
                json!({"type":"image", "mimeType":block["source"]["media_type"],
                "data":block["source"]["data"]}),
            ),
            _ => {}
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
    block_index: usize,
    tools: HashSet<String>,
    context_usage: TokenUsage,
    compacting: bool,
    child_tasks: HashSet<String>,
    agent_tools: HashSet<String>,
}

impl Events {
    pub(super) fn activity(&mut self, activity: WorkerActivity) {
        self.pending.push_back(WorkerEvent::Activity(activity));
    }

    pub(super) fn start(&mut self) {
        self.block_index = 0;
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

    fn child_task(&mut self, frame: &Value) {
        let task = string(frame, "task_id");
        let subtype = string(frame, "subtype");
        let is_agent = frame["task_type"] == "local_agent"
            || frame["subagent_type"].as_str().is_some()
            || self.child_tasks.contains(task)
            || frame["tool_use_id"]
                .as_str()
                .is_some_and(|id| self.agent_tools.contains(id));
        if is_agent && let Some(id) = super::catalog::child_id(string(frame, "session_id"), task) {
            self.child_tasks.insert(task.into());
            self.activity(WorkerActivity::ChildSessionsChanged {
                id,
                title: frame["description"].as_str().map(str::to_owned),
                is_running: subtype != "task_notification",
                outcome: None,
                execution: None,
            });
        }
    }

    pub(super) fn message(&mut self, frame: &Value) {
        if frame["parent_tool_use_id"].as_str().is_some() {
            return;
        }
        match string(frame, "type") {
            "stream_event" => {
                let event = &frame["event"];
                let index = event["index"].as_u64().unwrap_or(0) as usize;
                match string(event, "type") {
                    "message_start" => {
                        self.block_index = 0;
                        self.streamed.clear(); self.thinking.clear();
                        self.context_usage = tokens(&event["message"]["usage"]);
                    }
                    "message_delta" => {
                        let usage = &event["usage"];
                        for (field, count) in [
                            ("input_tokens", &mut self.context_usage.input),
                            ("output_tokens", &mut self.context_usage.output),
                            ("cache_read_input_tokens", &mut self.context_usage.cache_read),
                            ("cache_creation_input_tokens", &mut self.context_usage.cache_write),
                        ] {
                            if let Some(value) = usage[field].as_u64() {
                                *count = value;
                            }
                        }
                    }
                    "content_block_start" => {
                        self.block_index = index;
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
                let content = blocks(message);
                for (index, block) in content.iter().enumerate() {
                    let index = if content.len() == 1 { self.block_index } else { index };
                    match string(block,"type") {
                        "text" | "thinking" => {
                            let thinking = block["type"] == "thinking";
                            let full = string(block, if thinking {"thinking"} else {"text"});
                            let previous = if thinking {&self.thinking} else {&self.streamed}
                                .get(&index).map(String::as_str).unwrap_or_default();
                            if let TextUpdate::Append(suffix) = TextUpdate::between(previous, full) {
                                self.delta(index, suffix, thinking);
                            }
                        }
                        "tool_use" => {
                            let id = string(block,"id");
                            if matches!(string(block, "name"), "Agent" | "Task") {
                                self.agent_tools.insert(id.into());
                            }
                            if self.tools.insert(id.into()) {
                                self.activity(WorkerActivity::ToolStarted { id:id.into(),
                                    name:string(block,"name").into(), args:block["input"].clone(),
                                    metadata:tool_metadata(string(block,"name"), &block["input"]) });
                            }
                        }
                        _ => {},
                    }
                }
                if let [block] = content.as_ref()
                    && let Some(id) = frame["uuid"].as_str()
                    && matches!(block["type"].as_str(), Some("text" | "thinking"))
                {
                    self.activity(WorkerActivity::AssistantItem {
                        source: crate::MessageSource { id: id.into(), turn_id: None },
                        final_content: Some(block.clone()),
                    });
                }
            }
            "user" => {
                let blocks = blocks(&frame["message"]);
                let native = native_tool_result(frame, &blocks);
                for block in blocks.iter() {
                    if block["type"] == "tool_result" && self.tools.remove(string(block,"tool_use_id")) {
                        self.activity(WorkerActivity::ToolFinished { id:string(block,"tool_use_id").into(),
                            result:tool_result(block, native),
                            is_error:block["is_error"].as_bool().unwrap_or(false) });
                    }
                }
            }
            "result" => {
                let mut usage = WorkerUsage { turn:self.context_usage, ..Default::default() };
                for model in frame["modelUsage"].as_object().into_iter().flat_map(|models| models.values()) {
                    usage.session = usage.session.saturating_add(TokenUsage {
                        input: model["inputTokens"].as_u64().unwrap_or(0),
                        output: model["outputTokens"].as_u64().unwrap_or(0),
                        cache_read: model["cacheReadInputTokens"].as_u64().unwrap_or(0),
                        cache_write: model["cacheCreationInputTokens"].as_u64().unwrap_or(0),
                    });
                    usage.context_window = usage.context_window.max(model["contextWindow"].as_u64().unwrap_or(0));
                }
                usage.cost = frame["total_cost_usd"].as_f64().filter(|cost| cost.is_finite() && *cost >= 0.0);
                self.activity(WorkerActivity::Usage(usage));
            }
            "tool_progress" => self.activity(WorkerActivity::ToolUpdated {
                id:string(frame,"tool_use_id").into(),
                content:json!([{"type":"text","text":format!("Running for {}s", frame["elapsed_time_seconds"])}]) }),
            "system" => match string(frame,"subtype") {
                "task_started" | "task_progress" | "task_notification" => {
                    self.child_task(frame);
                }
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
