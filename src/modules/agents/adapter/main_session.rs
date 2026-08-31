use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
};

use serde_json::{Value, json};

use crate::agents::{
    SessionCommand, SessionEvent, SessionResponse, SessionTransport, TokenUsage, WorkerActivity,
    WorkerEvent, WorkerInput, WorkerInputResponse, WorkerSendMode, WorkerSession, WorkerUsage,
    extensions::{ExtensionUiRequest, ExtensionUiResponse, PromptMode},
};

#[derive(Default)]
pub(super) struct MainSessionMetadata {
    pub models: Vec<Value>,
    pub efforts: Vec<String>,
    pub commands: Vec<Value>,
    pub modes: Vec<Value>,
}

pub(super) struct WorkerSessionTransport {
    harness: String,
    locator: String,
    path: PathBuf,
    worker: Box<dyn WorkerSession>,
    pending: VecDeque<SessionEvent>,
    next_id: u64,
    running: bool,
    assistant_message: AssistantMessage,
    model: Option<(String, String)>,
    effort: String,
    metadata: MainSessionMetadata,
    selected_mode: Option<String>,
    usage: WorkerUsage,
}

impl WorkerSessionTransport {
    pub(super) fn new(
        locator_root: &std::path::Path,
        harness: &str,
        locator: String,
        worker: Box<dyn WorkerSession>,
        metadata: MainSessionMetadata,
    ) -> Result<Self, String> {
        let path = external_session_path(locator_root, harness, &locator);
        let model = metadata.models.first().and_then(|model| {
            Some((
                model.get("provider")?.as_str()?.to_owned(),
                model.get("id")?.as_str()?.to_owned(),
            ))
        });
        let selected_mode = metadata
            .modes
            .first()
            .and_then(|mode| mode.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let context_window = metadata
            .models
            .first()
            .and_then(|model| model.get("contextWindow"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        Ok(Self {
            harness: harness.into(),
            locator,
            path,
            worker,
            pending: VecDeque::new(),
            next_id: 0,
            running: false,
            assistant_message: AssistantMessage::default(),
            model,
            effort: metadata
                .efforts
                .first()
                .cloned()
                .unwrap_or_else(|| "off".into()),
            metadata,
            selected_mode,
            usage: WorkerUsage {
                context_window,
                ..WorkerUsage::default()
            },
        })
    }

    fn response(&mut self, id: String, command: &str, data: Value) {
        self.pending
            .push_back(SessionEvent::Response(SessionResponse {
                id: Some(id),
                command: command.into(),
                success: true,
                data,
                error: None,
            }));
    }

    fn state(&self) -> Value {
        let model = self.model.as_ref().map(|(provider, id)| {
            json!({
                "id": id,
                "name": id,
                "provider": provider,
                "contextWindow": self.usage.context_window,
                "reasoning": true,
            })
        });
        json!({
            "model": model,
            "thinkingLevel": self.effort,
            "isStreaming": self.running,
            "isCompacting": false,
            "sessionFile": self.path.to_string_lossy(),
            "sessionId": self.locator,
            "sessionName": null,
            "autoCompactionEnabled": true,
            "messageCount": 0,
            "pendingMessageCount": 0,
        })
    }

    fn enqueue_worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::Started => {
                self.running = true;
                self.assistant_message.clear();
                self.usage.turn = TokenUsage::default();
                self.pending
                    .push_back(SessionEvent::Activity(json!({"type": "agent_start"})));
            }
            WorkerEvent::Settled { output } => {
                self.running = false;
                if !self.assistant_message.started {
                    self.assistant_message.started = true;
                    self.pending.push_back(SessionEvent::Activity(json!({
                        "type": "message_start",
                        "message": {"role": "assistant", "content": []}
                    })));
                }
                if self.assistant_message.text().is_none() && !output.is_empty() {
                    let content_index = self.assistant_message.push_text(output.clone());
                    self.pending.push_back(SessionEvent::Activity(json!({
                        "type": "message_update",
                        "assistantMessageEvent": {
                            "type": "text_delta",
                            "contentIndex": content_index,
                            "delta": output,
                        }
                    })));
                } else {
                    self.assistant_message.replace_text(&output);
                }
                self.pending.push_back(SessionEvent::Activity(json!({
                    "type": "message_end",
                    "message": {
                        "role": "assistant",
                        "content": self.assistant_message.content(),
                        "usage": usage_json(self.usage.turn),
                    }
                })));
                self.pending
                    .push_back(SessionEvent::Activity(json!({"type": "agent_settled"})));
                self.assistant_message.clear();
            }
            WorkerEvent::SessionChanged { locator } => {
                self.locator = locator;
                self.pending.push_back(SessionEvent::Activity(
                    json!({"type": "session_info_changed"}),
                ));
            }
            WorkerEvent::NeedsInput(input) => {
                self.pending
                    .push_back(SessionEvent::Interaction(interaction(input)));
            }
            WorkerEvent::Activity(activity) => self.enqueue_activity(activity),
            WorkerEvent::Failed(error) => self.pending.push_back(SessionEvent::Failure(error)),
        }
    }

    fn enqueue_activity(&mut self, activity: WorkerActivity) {
        let event = match activity {
            WorkerActivity::TextDelta {
                content_index,
                delta,
            } => {
                self.start_assistant_message();
                self.assistant_message
                    .append_delta(content_index, "text", "text", &delta);
                json!({
                    "type": "message_update",
                    "assistantMessageEvent": {
                        "type": "text_delta",
                        "contentIndex": content_index,
                        "delta": delta,
                    }
                })
            }
            WorkerActivity::ThinkingDelta {
                content_index,
                delta,
            } => {
                self.start_assistant_message();
                self.assistant_message
                    .append_delta(content_index, "thinking", "thinking", &delta);
                json!({
                    "type": "message_update",
                    "assistantMessageEvent": {
                        "type": "thinking_delta",
                        "contentIndex": content_index,
                        "delta": delta,
                    }
                })
            }
            WorkerActivity::ToolStarted { id, name, args } => json!({
                "type": "tool_execution_start",
                "toolCallId": id,
                "toolName": name,
                "args": args,
            }),
            WorkerActivity::ToolUpdated { id, content } => json!({
                "type": "tool_execution_update",
                "toolCallId": id,
                "partialResult": {"content": content},
            }),
            WorkerActivity::ToolFinished {
                id,
                result,
                is_error,
            } => json!({
                "type": "tool_execution_end",
                "toolCallId": id,
                "result": {"content": result},
                "isError": is_error,
            }),
            WorkerActivity::Usage(usage) => {
                self.usage = usage;
                json!({
                    "type": "turn_end",
                    "contextWindow": usage.context_window,
                    "usage": usage_json(usage.turn),
                })
            }
            WorkerActivity::CompactionStarted => json!({
                "type": "compaction_start",
                "reason": "manual",
            }),
            WorkerActivity::CompactionFinished { aborted, error } => json!({
                "type": "compaction_end",
                "reason": "manual",
                "aborted": aborted,
                "errorMessage": error,
            }),
        };
        self.pending.push_back(SessionEvent::Activity(event));
    }

    fn start_assistant_message(&mut self) {
        if self.assistant_message.started {
            return;
        }
        self.assistant_message.started = true;
        self.pending.push_back(SessionEvent::Activity(json!({
            "type": "message_start",
            "message": {"role": "assistant", "content": []}
        })));
    }
}

impl SessionTransport for WorkerSessionTransport {
    fn send(&mut self, command: SessionCommand) -> Result<String, String> {
        self.next_id = self.next_id.saturating_add(1);
        let id = format!("{}-{}", self.harness, self.next_id);
        match command {
            SessionCommand::ConfigureSteering => {
                self.response(id.clone(), "set_steering_mode", json!({}))
            }
            SessionCommand::LoadState => {
                self.response(id.clone(), "get_state", self.state());
            }
            SessionCommand::LoadHistory => self.response(
                id.clone(),
                "get_entries",
                json!({"entries": [], "preserve": true}),
            ),
            SessionCommand::LoadUsage => self.response(
                id.clone(),
                "get_session_stats",
                json!({
                    "contextUsage": {
                        "tokens": self.usage.turn.total(),
                        "contextWindow": self.usage.context_window,
                        "percent": if self.usage.context_window > 0 {
                            self.usage.turn.total() as f64 * 100.0 / self.usage.context_window as f64
                        } else {
                            0.0
                        },
                    },
                    "tokens": usage_json(self.usage.session),
                }),
            ),
            SessionCommand::ListModels => self.response(
                id.clone(),
                "get_available_models",
                json!({"models": self.metadata.models.clone()}),
            ),
            SessionCommand::ListReasoningLevels => self.response(
                id.clone(),
                "get_available_thinking_levels",
                json!({"levels": self.metadata.efforts.clone()}),
            ),
            SessionCommand::ListModes => self.response(
                id.clone(),
                "get_modes",
                json!({"modes": self.metadata.modes.clone(), "selected": self.selected_mode}),
            ),
            SessionCommand::ListCommands => self.response(
                id.clone(),
                "get_commands",
                json!({"commands": self.metadata.commands.clone()}),
            ),
            SessionCommand::Prompt {
                mode,
                message,
                images,
            } => {
                let worker_mode = match mode {
                    PromptMode::Normal => WorkerSendMode::Prompt,
                    PromptMode::Steer => WorkerSendMode::Steer,
                    PromptMode::FollowUp => WorkerSendMode::Queue,
                };
                self.worker.send_with_images(message, worker_mode, images)?;
                let command = match mode {
                    PromptMode::Normal => "prompt",
                    PromptMode::Steer => "steer",
                    PromptMode::FollowUp => "follow_up",
                };
                self.response(id.clone(), command, json!({}));
            }
            SessionCommand::Abort => {
                self.worker.abort()?;
                self.response(id.clone(), "abort", json!({}));
            }
            SessionCommand::SelectModel { provider, model_id } => {
                self.worker.select_model(&provider, &model_id)?;
                self.model = Some((provider.clone(), model_id.clone()));
                self.response(
                    id.clone(),
                    "set_model",
                    json!({"id": model_id, "name": model_id, "provider": provider, "contextWindow": 0, "reasoning": true}),
                );
            }
            SessionCommand::SelectReasoning { level } => {
                self.worker.select_effort(&level)?;
                self.effort = level;
                self.response(id.clone(), "set_thinking_level", json!({}));
            }
            SessionCommand::SelectMode { mode } => {
                self.worker.select_mode(&mode)?;
                self.selected_mode = Some(mode);
                self.response(id.clone(), "set_mode", json!({}));
            }
            SessionCommand::Compact { .. } => {
                self.worker.compact()?;
                self.response(id.clone(), "compact", json!({}));
            }
            SessionCommand::Rename { name } => {
                self.worker.rename(&name)?;
                self.response(id.clone(), "set_session_name", json!({}));
            }
            SessionCommand::ExportHtml { .. } | SessionCommand::ForkAt { .. } => {
                return Err(format!(
                    "{} does not expose this command through its main-session bridge yet",
                    self.harness
                ));
            }
        }
        Ok(id)
    }

    fn respond(&mut self, response: ExtensionUiResponse) -> Result<(), String> {
        let response = match response {
            ExtensionUiResponse::Value { id, value } => WorkerInputResponse {
                id,
                value: Some(value),
                cancel: false,
            },
            ExtensionUiResponse::Confirmed { id, confirmed } => WorkerInputResponse {
                id,
                value: Some(if confirmed { "allow" } else { "decline" }.into()),
                cancel: false,
            },
            ExtensionUiResponse::Cancelled { id, .. } => WorkerInputResponse {
                id,
                value: None,
                cancel: true,
            },
        };
        self.worker.respond(response)
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        if let Some(event) = self.pending.pop_front() {
            return Some(event);
        }
        let event = self.worker.poll()?;
        self.enqueue_worker_event(event);
        self.pending.pop_front()
    }

    fn close(&mut self) -> Result<(), String> {
        self.worker.close()
    }
}

#[derive(Default)]
struct AssistantMessage {
    started: bool,
    content: BTreeMap<usize, Value>,
}

impl AssistantMessage {
    fn clear(&mut self) {
        self.started = false;
        self.content.clear();
    }

    fn append_delta(&mut self, index: usize, kind: &str, field: &str, delta: &str) {
        let part = self
            .content
            .entry(index)
            .or_insert_with(|| json!({"type": kind, field: ""}));
        append_content_text(part, field, delta);
    }

    fn text(&self) -> Option<&str> {
        self.content.values().rev().find_map(|part| {
            (part.get("type").and_then(Value::as_str) == Some("text"))
                .then(|| part.get("text").and_then(Value::as_str))
                .flatten()
        })
    }

    fn push_text(&mut self, text: String) -> usize {
        let index = self
            .content
            .last_key_value()
            .map_or(0, |(index, _)| index + 1);
        self.content
            .insert(index, json!({"type": "text", "text": text}));
        index
    }

    fn replace_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some((_, part)) = self
            .content
            .iter_mut()
            .rev()
            .find(|(_, part)| part.get("type").and_then(Value::as_str) == Some("text"))
        {
            part["text"] = Value::String(text.to_owned());
        }
    }

    fn content(&self) -> Vec<Value> {
        self.content.values().cloned().collect()
    }
}

fn append_content_text(part: &mut Value, field: &str, delta: &str) {
    match part.get_mut(field) {
        Some(Value::String(text)) => text.push_str(delta),
        _ => part[field] = Value::String(delta.to_owned()),
    }
}

fn usage_json(usage: TokenUsage) -> Value {
    json!({
        "input": usage.input,
        "output": usage.output,
        "cacheRead": usage.cache_read,
        "cacheWrite": usage.cache_write,
        "totalTokens": usage.total(),
    })
}

fn interaction(input: WorkerInput) -> ExtensionUiRequest {
    if input.options.is_empty() {
        ExtensionUiRequest::Input {
            id: input.id,
            title: input.prompt,
            placeholder: None,
            timeout: None,
        }
    } else if input.options.len() == 2 {
        ExtensionUiRequest::Confirm {
            id: input.id,
            title: input.prompt.clone(),
            message: input.prompt,
            timeout: None,
        }
    } else {
        ExtensionUiRequest::Select {
            id: input.id,
            title: input.prompt,
            options: input.options,
            timeout: None,
        }
    }
}

pub(in crate::modules::agents::adapter) fn external_session_path(
    locator_root: &std::path::Path,
    harness: &str,
    locator: &str,
) -> PathBuf {
    let encoded = url::form_urlencoded::byte_serialize(locator.as_bytes()).collect::<String>();
    locator_root.join(harness).join(encoded)
}

pub(in crate::modules::agents::adapter) fn external_session_locator(
    harness: &str,
    path: &std::path::Path,
) -> Option<String> {
    (path.parent()?.file_name()?.to_str()? == harness)
        .then(|| percent_decode(path.file_name()?.to_str()?))
        .flatten()
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex(bytes.get(index + 1).copied()?)?;
            let low = hex(bytes.get(index + 2).copied()?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

const fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_is_an_authoritative_message_before_settling() {
        let mut message = AssistantMessage::default();
        message.append_delta(0, "thinking", "thinking", "plan");
        message.append_delta(1, "text", "text", "partial");
        message.replace_text("final");
        assert_eq!(
            message.content(),
            vec![
                json!({"type": "thinking", "thinking": "plan"}),
                json!({"type": "text", "text": "final"}),
            ]
        );
    }
}
