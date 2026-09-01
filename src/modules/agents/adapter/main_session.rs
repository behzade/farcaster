use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
};

use serde_json::{Value, json};

use crate::agents::{
    SessionCommand, SessionEvent, SessionOperation, SessionResponse, SessionTransport, TokenUsage,
    WorkerActivity, WorkerEvent, WorkerInput, WorkerInputResponse, WorkerSendMode, WorkerSession,
    WorkerUsage,
    extensions::{ExtensionUiRequest, ExtensionUiResponse, PromptMode},
};

#[derive(Default)]
pub(super) struct MainSessionMetadata {
    pub models: Vec<Value>,
    pub efforts: Vec<String>,
    pub commands: Vec<Value>,
    pub modes: Vec<Value>,
}

fn activity(value: Value) -> SessionEvent {
    SessionEvent::Activity(value.into())
}

pub(super) struct WorkerSessionTransport {
    harness: String,
    locator: String,
    path: PathBuf,
    worker: Box<dyn WorkerSession>,
    pending: VecDeque<SessionEvent>,
    next_id: u64,
    running: bool,
    steering: Vec<String>,
    follow_up: Vec<String>,
    assistant_message: AssistantMessage,
    observed_text: String,
    model: Option<(String, String)>,
    effort: String,
    metadata: MainSessionMetadata,
    history: Option<Vec<Value>>,
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
        history: Option<crate::agents::DiscoveredHistory>,
    ) -> Result<Self, String> {
        let path = external_session_path(locator_root, harness, &locator);
        let model = history
            .as_ref()
            .and_then(|history| history.model.clone())
            .or_else(|| {
                metadata.models.first().and_then(|model| {
                    Some((
                        model.get("provider")?.as_str()?.to_owned(),
                        model.get("id")?.as_str()?.to_owned(),
                    ))
                })
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
            steering: Vec::new(),
            follow_up: Vec::new(),
            assistant_message: AssistantMessage::default(),
            observed_text: String::new(),
            model,
            effort: history
                .as_ref()
                .and_then(|history| history.thinking_level.clone())
                .or_else(|| metadata.efforts.first().cloned())
                .unwrap_or_else(|| "off".into()),
            metadata,
            history: history.map(|history| history.messages),
            selected_mode,
            usage: WorkerUsage {
                context_window,
                ..WorkerUsage::default()
            },
        })
    }

    fn response(&mut self, id: String, operation: SessionOperation, data: Value) {
        self.pending
            .push_back(SessionEvent::Response(SessionResponse {
                id: Some(id),
                operation,
                success: true,
                data,
                error: None,
            }));
    }

    fn enqueue_queue_update(&mut self) {
        self.pending.push_back(activity(json!({
            "type": "queue_update",
            "steering": self.steering,
            "followUp": self.follow_up,
        })));
    }

    fn enqueue_message(&mut self, mode: PromptMode, message: String) {
        match mode {
            PromptMode::Normal => return,
            PromptMode::Steer => self.steering.push(message),
            PromptMode::FollowUp => self.follow_up.push(message),
        }
        self.enqueue_queue_update();
    }

    fn clear_queue(&mut self) {
        if self.steering.is_empty() && self.follow_up.is_empty() {
            return;
        }
        self.steering.clear();
        self.follow_up.clear();
        self.enqueue_queue_update();
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
            "messageCount": self.history.as_ref().map_or(0, Vec::len),
            "pendingMessageCount": 0,
        })
    }

    fn enqueue_worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::Started => {
                self.running = true;
                self.assistant_message.clear();
                self.observed_text.clear();
                self.usage.turn = TokenUsage::default();
                self.pending
                    .push_back(activity(json!({"type": "agent_start"})));
            }
            WorkerEvent::Settled { output } => {
                self.running = false;
                self.reconcile_completed_output(&output);
                self.start_assistant_message();
                self.finish_assistant_message(Some(self.usage.turn));
                self.clear_queue();
                self.pending
                    .push_back(activity(json!({"type": "agent_settled"})));
                self.assistant_message.clear();
                self.observed_text.clear();
            }
            WorkerEvent::SessionChanged { locator } => {
                self.locator = locator;
                self.pending
                    .push_back(activity(json!({"type": "session_info_changed"})));
            }
            WorkerEvent::NeedsInput(input) => {
                self.pending
                    .push_back(SessionEvent::Interaction(interaction(input)));
            }
            WorkerEvent::Activity(activity) => self.enqueue_activity(activity),
            WorkerEvent::Failed(error) => {
                self.clear_queue();
                self.pending.push_back(SessionEvent::Failure(error));
            }
        }
    }

    fn enqueue_activity(&mut self, worker_activity: WorkerActivity) {
        let event = match worker_activity {
            WorkerActivity::TextDelta {
                content_index,
                delta,
            } => {
                self.start_assistant_message();
                self.assistant_message
                    .append_delta(content_index, "text", "text", &delta);
                self.observed_text.push_str(&delta);
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
            WorkerActivity::ToolStarted { id, name, args } => {
                self.finish_assistant_message(None);
                json!({
                    "type": "tool_execution_start",
                    "toolCallId": id,
                    "toolName": name,
                    "args": args,
                })
            }
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
        self.pending.push_back(activity(event));
    }

    fn start_assistant_message(&mut self) {
        if self.assistant_message.started {
            return;
        }
        self.assistant_message.started = true;
        self.pending.push_back(activity(json!({
            "type": "message_start",
            "message": {"role": "assistant", "content": []}
        })));
    }

    fn finish_assistant_message(&mut self, usage: Option<TokenUsage>) {
        if !self.assistant_message.started {
            return;
        }
        let mut message = json!({
            "role": "assistant",
            "content": self.assistant_message.content(),
        });
        if let Some(usage) = usage {
            message["usage"] = usage_json(usage);
        }
        self.pending.push_back(activity(json!({
            "type": "message_end",
            "message": message,
        })));
        self.assistant_message.clear();
    }

    fn reconcile_completed_output(&mut self, output: &str) {
        let current_text = self.assistant_message.text().unwrap_or_default();
        if output.is_empty() || output == self.observed_text || output == current_text {
            return;
        }
        if let Some(suffix) = output.strip_prefix(&self.observed_text) {
            self.append_completed_text(suffix);
        } else if current_text.is_empty() {
            self.append_completed_text(output);
        } else {
            self.assistant_message.replace_text(output);
        }
    }

    fn append_completed_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.start_assistant_message();
        let content_index = self.assistant_message.append_text(text);
        self.pending.push_back(activity(json!({
            "type": "message_update",
            "assistantMessageEvent": {
                "type": "text_delta",
                "contentIndex": content_index,
                "delta": text,
            }
        })));
        self.observed_text.push_str(text);
    }
}

impl SessionTransport for WorkerSessionTransport {
    fn send(&mut self, command: SessionCommand) -> Result<String, String> {
        self.next_id = self.next_id.saturating_add(1);
        let id = format!("{}-{}", self.harness, self.next_id);
        let operation = command.response_operation();
        match command {
            SessionCommand::ConfigureSteering => {
                self.response(id.clone(), operation, json!({}))
            }
            SessionCommand::LoadState => {
                self.response(id.clone(), operation, self.state());
            }
            SessionCommand::LoadHistory => {
                let data = self.history.as_ref().map_or_else(
                    || json!({"entries": [], "preserve": true}),
                    |messages| {
                        let entries = messages
                            .iter()
                            .map(|message| json!({"type": "message", "message": message}))
                            .collect::<Vec<_>>();
                        json!({"entries": entries, "preserve": false})
                    },
                );
                self.response(id.clone(), operation, data);
            }
            SessionCommand::LoadUsage => self.response(
                id.clone(),
                operation,
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
                operation,
                json!({"models": self.metadata.models.clone()}),
            ),
            SessionCommand::ListReasoningLevels => self.response(
                id.clone(),
                operation,
                json!({"levels": self.metadata.efforts.clone()}),
            ),
            SessionCommand::ListModes => self.response(
                id.clone(),
                operation,
                json!({"modes": self.metadata.modes.clone(), "selected": self.selected_mode}),
            ),
            SessionCommand::ListCommands => self.response(
                id.clone(),
                operation,
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
                let queued_message = (mode != PromptMode::Normal).then(|| message.clone());
                self.worker.send_with_images(message, worker_mode, images)?;
                if let Some(message) = queued_message {
                    self.enqueue_message(mode, message);
                }
                self.response(id.clone(), operation, json!({}));
            }
            SessionCommand::Abort => {
                self.worker.abort()?;
                self.clear_queue();
                self.response(id.clone(), operation, json!({}));
            }
            SessionCommand::SelectModel { provider, model_id } => {
                self.worker.select_model(&provider, &model_id)?;
                self.model = Some((provider.clone(), model_id.clone()));
                self.response(
                    id.clone(),
                    operation,
                    json!({"id": model_id, "name": model_id, "provider": provider, "contextWindow": 0, "reasoning": true}),
                );
            }
            SessionCommand::SelectReasoning { level } => {
                self.worker.select_effort(&level)?;
                self.effort = level;
                self.response(id.clone(), operation, json!({}));
            }
            SessionCommand::SelectMode { mode } => {
                self.worker.select_mode(&mode)?;
                self.selected_mode = Some(mode);
                self.response(id.clone(), operation, json!({}));
            }
            SessionCommand::Compact { .. } => {
                self.worker.compact()?;
                self.response(id.clone(), operation, json!({}));
            }
            SessionCommand::Rename { name } => {
                self.worker.rename(&name)?;
                self.response(id.clone(), operation, json!({}));
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

    fn append_text(&mut self, text: &str) -> usize {
        if let Some((index, part)) = self
            .content
            .iter_mut()
            .rev()
            .find(|(_, part)| part.get("type").and_then(Value::as_str) == Some("text"))
        {
            append_content_text(part, "text", text);
            return *index;
        }
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

pub(in crate::modules::agents::adapter) fn launch_session_locator(
    launch: &crate::agents::SessionLaunch,
) -> Option<String> {
    match &launch.start {
        crate::agents::SessionStart::New => launch.session_id.clone(),
        crate::agents::SessionStart::Resume(path) | crate::agents::SessionStart::Fork(path) => {
            external_session_locator(&launch.harness, path).or_else(|| launch.session_id.clone())
        }
    }
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

    struct IdleWorker;

    impl WorkerSession for IdleWorker {
        fn send(&mut self, _: String, _: WorkerSendMode) -> Result<(), String> {
            Ok(())
        }

        fn respond(&mut self, _: WorkerInputResponse) -> Result<(), String> {
            Ok(())
        }

        fn abort(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn poll(&mut self) -> Option<WorkerEvent> {
            None
        }

        fn close(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn resume_locator_comes_from_the_external_session_path_when_the_runtime_has_no_id() {
        let path = external_session_path(
            std::path::Path::new("/locators"),
            "opencode2",
            "session/one",
        );
        let launch = crate::agents::SessionLaunch {
            harness: "opencode2".into(),
            session_id: None,
            project: "/project".into(),
            start: crate::agents::SessionStart::Resume(path),
            wake: None,
        };

        assert_eq!(
            launch_session_locator(&launch).as_deref(),
            Some("session/one")
        );
    }

    #[test]
    fn text_around_tools_is_emitted_as_chronological_messages() {
        let mut transport = WorkerSessionTransport::new(
            std::path::Path::new("/locators"),
            "codex-cli",
            "thread-1".into(),
            Box::new(IdleWorker),
            MainSessionMetadata::default(),
            None,
        )
        .expect("transport");

        transport.enqueue_worker_event(WorkerEvent::Started);
        transport.enqueue_worker_event(WorkerEvent::Activity(WorkerActivity::TextDelta {
            content_index: 0,
            delta: "before".into(),
        }));
        transport.enqueue_worker_event(WorkerEvent::Activity(WorkerActivity::ToolStarted {
            id: "tool-1".into(),
            name: "command".into(),
            args: json!({}),
        }));
        transport.enqueue_worker_event(WorkerEvent::Activity(WorkerActivity::TextDelta {
            content_index: 0,
            delta: "after".into(),
        }));
        transport.enqueue_worker_event(WorkerEvent::Settled {
            output: "beforeafter".into(),
        });

        let activities = transport
            .pending
            .into_iter()
            .filter_map(|event| match event {
                SessionEvent::Activity(activity) => Some(activity),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            activities
                .iter()
                .map(|activity| activity.value()["type"].as_str().unwrap_or_default())
                .collect::<Vec<_>>(),
            [
                "agent_start",
                "message_start",
                "message_update",
                "message_end",
                "tool_execution_start",
                "message_start",
                "message_update",
                "message_end",
                "agent_settled",
            ]
        );
        assert_eq!(
            activities[3].value()["message"]["content"][0]["text"],
            "before"
        );
        assert_eq!(
            activities[7].value()["message"]["content"][0]["text"],
            "after"
        );
    }

    #[test]
    fn queued_worker_messages_are_reported_until_the_turn_settles() {
        let mut transport = WorkerSessionTransport::new(
            std::path::Path::new("/locators"),
            "codex-cli",
            "thread-1".into(),
            Box::new(IdleWorker),
            MainSessionMetadata::default(),
            None,
        )
        .expect("transport");

        transport
            .send(SessionCommand::Prompt {
                mode: PromptMode::Steer,
                message: "redirect".into(),
                images: Vec::new(),
            })
            .expect("steer");
        transport
            .send(SessionCommand::Prompt {
                mode: PromptMode::FollowUp,
                message: "then verify".into(),
                images: Vec::new(),
            })
            .expect("follow-up");
        transport.enqueue_worker_event(WorkerEvent::Settled {
            output: String::new(),
        });

        let updates = transport
            .pending
            .iter()
            .filter_map(|event| match event {
                SessionEvent::Activity(activity)
                    if activity.kind() == &crate::agents::SessionActivityKind::QueueUpdated =>
                {
                    Some(activity.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(updates.len(), 3);
        assert_eq!(updates[0].value()["steering"], json!(["redirect"]));
        assert_eq!(updates[0].value()["followUp"], json!([]));
        assert_eq!(updates[1].value()["steering"], json!(["redirect"]));
        assert_eq!(updates[1].value()["followUp"], json!(["then verify"]));
        assert_eq!(updates[2].value()["steering"], json!([]));
        assert_eq!(updates[2].value()["followUp"], json!([]));
    }

    #[test]
    fn resumed_transport_returns_persisted_history() {
        let history = crate::agents::DiscoveredHistory {
            messages: vec![json!({"role": "user", "content": "persisted"})],
            model: Some(("openai".into(), "gpt-test".into())),
            thinking_level: Some("high".into()),
        };
        let mut transport = WorkerSessionTransport::new(
            std::path::Path::new("/locators"),
            "codex-cli",
            "thread-1".into(),
            Box::new(IdleWorker),
            MainSessionMetadata::default(),
            Some(history),
        )
        .expect("transport");

        transport
            .send(SessionCommand::LoadHistory)
            .expect("load history");
        let SessionEvent::Response(response) = transport.poll().expect("history response") else {
            panic!("expected history response");
        };
        assert_eq!(response.operation, SessionOperation::LoadHistory);
        assert_eq!(response.data["preserve"], false);
        assert_eq!(
            response.data["entries"][0]["message"]["content"],
            "persisted"
        );

        transport
            .send(SessionCommand::LoadState)
            .expect("load state");
        let SessionEvent::Response(response) = transport.poll().expect("state response") else {
            panic!("expected state response");
        };
        assert_eq!(response.data["messageCount"], 1);
        assert_eq!(response.data["model"]["id"], "gpt-test");
        assert_eq!(response.data["thinkingLevel"], "high");
    }

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
