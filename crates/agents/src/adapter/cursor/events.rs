use crate::{TokenUsage, ToolMetadata, WorkerActivity};
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct Events {
    pub(super) output: String,
    index: usize,
    segment: Option<Segment>,
    tools: HashMap<String, (String, bool)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Segment {
    Text,
    Thinking,
}

impl Events {
    fn end_segment(&mut self) {
        if self.segment.take().is_some() {
            self.index += 1;
        }
    }

    fn start_segment(&mut self, segment: Segment) -> bool {
        if self.segment == Some(segment) {
            return false;
        }
        self.end_segment();
        self.segment = Some(segment);
        true
    }

    pub(super) fn interaction(&mut self, update: &Value) -> Vec<WorkerActivity> {
        match update["type"].as_str().unwrap_or_default() {
            "text-delta" => self.message(
                "assistant",
                &json!({"message":{"content":[{"type":"text","text":update["text"]}]}}),
            ),
            "thinking-delta" => self.message("thinking", &json!({"text":update["text"]})),
            "thinking-completed" | "step-completed" | "turn-ended" => {
                self.end_segment();
                Vec::new()
            }
            "step-started" => {
                self.end_segment();
                vec![WorkerActivity::TurnStarted]
            }
            kind @ ("tool-call-started" | "partial-tool-call" | "tool-call-completed") => {
                let tool = &update["toolCall"];
                self.message("tool_call", &json!({
                    "call_id":update["callId"], "name":tool["type"], "args":tool["args"],
                    "status": if kind != "tool-call-completed" { "running" } else if tool_failed(&tool["result"]) { "error" } else { "completed" },
                    "result":tool["result"]
                }))
            }
            "shell-output-delta" => {
                let event = &update["event"];
                if !matches!(event["case"].as_str(), Some("stdout" | "stderr")) {
                    return Vec::new();
                }
                let Some(text) = event.pointer("/value/data").and_then(Value::as_str) else {
                    return Vec::new();
                };
                // The SDK omits a call ID here. Never assign output to a finished
                // tool or guess between concurrent shell calls.
                let mut shells = self
                    .tools
                    .iter()
                    .filter(|(_, (name, done))| name == "shell" && !done);
                let Some((id, _)) = shells.next() else {
                    return Vec::new();
                };
                if shells.next().is_some() || text.is_empty() {
                    return Vec::new();
                }
                vec![WorkerActivity::ToolUpdated {
                    id: id.clone(),
                    content: json!([{"type":"text","text":text}]),
                }]
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn message(&mut self, kind: &str, payload: &Value) -> Vec<WorkerActivity> {
        match kind {
            "assistant" => {
                let text = payload
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|p| p["type"] == "text")
                    .filter_map(|p| p["text"].as_str())
                    .collect::<String>();
                if text.is_empty() {
                    return Vec::new();
                }
                // SDK assistant messages carry incremental fragments, not snapshots.
                self.start_segment(Segment::Text);
                self.output.push_str(&text);
                vec![WorkerActivity::TextDelta {
                    content_index: self.index,
                    delta: text,
                }]
            }
            "thinking" => {
                let text = payload["text"].as_str().unwrap_or_default();
                let mut events = Vec::new();
                if !text.is_empty() {
                    if self.start_segment(Segment::Thinking) {
                        events.push(WorkerActivity::ThinkingStarted {
                            content_index: self.index,
                        });
                    }
                    events.push(WorkerActivity::ThinkingDelta {
                        content_index: self.index,
                        delta: text.into(),
                    });
                }
                if payload["thinking_duration_ms"].is_number()
                    && self.segment == Some(Segment::Thinking)
                {
                    self.end_segment();
                }
                events
            }
            "tool_call" => {
                let Some(id) = payload["call_id"].as_str() else {
                    return Vec::new();
                };
                self.end_segment();
                let name = payload["name"].as_str().unwrap_or("tool");
                let metadata = ToolMetadata {
                    native: Some(payload.clone()),
                    ..Default::default()
                };
                match payload["status"].as_str() {
                    Some("running") => {
                        if let Some((_, finished)) = self.tools.get(id) {
                            if *finished {
                                return Vec::new();
                            }
                            return vec![WorkerActivity::ToolMetadataChanged {
                                id: id.into(),
                                args: payload.get("args").cloned(),
                                metadata,
                            }];
                        }
                        self.tools.insert(id.into(), (name.into(), false));
                        vec![WorkerActivity::ToolStarted {
                            id: id.into(),
                            name: name.into(),
                            args: payload["args"].clone(),
                            metadata,
                        }]
                    }
                    Some(status @ ("completed" | "error")) => {
                        if self.tools.get(id).is_some_and(|(_, done)| *done) {
                            return Vec::new();
                        }
                        let known = self.tools.insert(id.into(), (name.into(), true)).is_some();
                        let mut events = vec![if known {
                            WorkerActivity::ToolMetadataChanged {
                                id: id.into(),
                                args: payload.get("args").cloned(),
                                metadata,
                            }
                        } else {
                            WorkerActivity::ToolStarted {
                                id: id.into(),
                                name: name.into(),
                                args: payload["args"].clone(),
                                metadata,
                            }
                        }];
                        events.push(WorkerActivity::ToolFinished {
                            id: id.into(),
                            result: tool_result(&payload["result"]),
                            is_error: status == "error",
                        });
                        events
                    }
                    _ => Vec::new(),
                }
            }

            _ => Vec::new(),
        }
    }
}

pub(super) fn tool_failed(value: &Value) -> bool {
    let status = value["status"].as_str();
    status.is_some_and(|s| s != "success")
        || value
            .pointer("/value/exitCode")
            .and_then(Value::as_i64)
            .is_some_and(|code| code != 0)
}

pub(super) fn tool_result(value: &Value) -> Value {
    let value = if value["status"] == "success" {
        &value["value"]
    } else if value["status"] == "error" {
        &value["error"]
    } else {
        value
    };
    if value.get("stdout").is_some() || value.get("stderr").is_some() {
        return json!([{"type":"text","text": format!("{}{}", value["stdout"].as_str().unwrap_or_default(), value["stderr"].as_str().unwrap_or_default())}]);
    }
    if let Some(text) = value.get("content").and_then(Value::as_str) {
        return json!([{"type":"text","text":text}]);
    }
    if value.is_array() || value.get("content").is_some() {
        value.clone()
    } else {
        json!([{"type":"text","text":value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string())}])
    }
}

pub(super) fn usage(value: &Value) -> TokenUsage {
    let count = |key| {
        value[key]
            .as_u64()
            .or_else(|| value[key].as_str()?.parse().ok())
            .unwrap_or(0)
    };
    TokenUsage {
        input: count("inputTokens"),
        output: count("outputTokens"),
        cache_read: count("cacheReadTokens"),
        cache_write: count("cacheWriteTokens"),
    }
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
