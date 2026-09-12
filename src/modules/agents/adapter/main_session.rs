mod responses;
use responses::CatalogQuery;

use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
};

use serde_json::{Value, json};

use crate::agents::{
    SessionCommand, SessionEvent, SessionHistory, SessionOperation, SessionResponse,
    SessionResponsePayload as Payload, SessionTransport, TokenUsage, ToolReviewState,
    WorkerActivity, WorkerEvent, WorkerInput, WorkerInputResponse, WorkerSendMode, WorkerSession,
    WorkerUsage,
    extensions::{ExtensionUiRequest, ExtensionUiResponse, Model, PromptMode},
};

#[derive(Default)]
pub(super) struct MainSessionMetadata {
    pub session_name: Option<String>,
    pub service_tier: Option<String>,
    pub service_tiers: Vec<String>,
    pub models: Vec<Value>,
    pub efforts: Vec<String>,
    pub commands: Vec<Value>,
    pub modes: Vec<Value>,
}

fn activity(value: Value) -> SessionEvent {
    SessionEvent::Activity(value.into())
}

fn finished_tool_result(result: Value) -> Value {
    if result.get("content").is_some() || result.get("details").is_some() {
        result
    } else {
        json!({"content": result})
    }
}

struct PendingPrompt {
    requested_mode: PromptMode,
    delivery_mode: PromptMode,
    queued_message: Option<String>,
}

pub(super) struct WorkerSessionTransport {
    harness: String,
    locator: String,
    path: PathBuf,
    worker: Box<dyn WorkerSession>,
    pending: VecDeque<SessionEvent>,
    next_id: u64,
    pending_prompts: BTreeMap<String, PendingPrompt>,
    running: bool,
    steering: Vec<String>,
    follow_up: Vec<String>,
    assistant_message: AssistantMessage,
    observed_text: String,
    model: Option<(String, String)>,
    effort: Option<String>,
    metadata: MainSessionMetadata,
    history: Option<Vec<Value>>,
    message_count: usize,
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
            pending_prompts: BTreeMap::new(),
            running: false,
            steering: Vec::new(),
            follow_up: Vec::new(),
            assistant_message: AssistantMessage::default(),
            observed_text: String::new(),
            model,
            effort: history
                .as_ref()
                .and_then(|history| history.thinking_level.clone())
                .filter(|level| !level.is_empty())
                .or_else(|| metadata.efforts.first().cloned()),
            metadata,
            message_count: history.as_ref().map_or(0, |history| history.messages.len()),
            history: history.map(|history| history.messages),
            selected_mode,
            usage: WorkerUsage {
                context_window,
                ..WorkerUsage::default()
            },
        })
    }

    fn finish_prompt_ack(&mut self, id: String, result: Result<(), String>) {
        let Some(PendingPrompt {
            requested_mode,
            delivery_mode,
            queued_message,
        }) = self.pending_prompts.remove(&id)
        else {
            return;
        };
        if result.is_ok() {
            self.message_count = self.message_count.saturating_add(1);
            if let Some(message) = queued_message {
                self.enqueue_message(delivery_mode, message);
            }
        }
        let response = match result {
            Ok(()) => SessionResponse::success(Some(id), Payload::Prompt(requested_mode)),
            Err(error) => {
                SessionResponse::failure(Some(id), SessionOperation::Prompt(requested_mode), error)
            }
        };
        self.pending.push_back(SessionEvent::Response(response));
    }

    fn drain_prompt_acks(&mut self) {
        while let Some((id, result)) = self.worker.poll_prompt_ack() {
            self.finish_prompt_ack(id, result);
        }
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

    fn acknowledge_delivery(&mut self, mode: WorkerSendMode, message: &str) {
        let queue = match mode {
            WorkerSendMode::Prompt => return,
            WorkerSendMode::Steer => &mut self.steering,
            WorkerSendMode::Queue => &mut self.follow_up,
        };
        if let Some(index) = queue.iter().position(|queued| queued == message) {
            queue.remove(index);
            self.enqueue_queue_update();
        }
    }

    fn clear_queue(&mut self) {
        if self.steering.is_empty() && self.follow_up.is_empty() {
            return;
        }
        self.steering.clear();
        self.follow_up.clear();
        self.enqueue_queue_update();
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
                for id in self.pending_prompts.keys().cloned().collect::<Vec<_>>() {
                    self.finish_prompt_ack(id, Err(error.clone()));
                }
                self.clear_queue();
                self.pending.push_back(SessionEvent::Failure(error));
            }
        }
    }

    fn enqueue_activity(&mut self, worker_activity: WorkerActivity) {
        let event = match worker_activity {
            WorkerActivity::InputDelivered { mode, message } => {
                self.input_delivered(mode, &message, json!(message));
                return;
            }
            WorkerActivity::InputDeliveredWithImages {
                mode,
                message,
                images,
            } => {
                let mut content = vec![json!({"type":"text","text":message})];
                content.extend(images.into_iter().map(|image| {
                    json!({
                        "type":"image", "data":image.data, "mimeType":image.mime_type,
                    })
                }));
                self.input_delivered(mode, &message, json!(content));
                return;
            }
            WorkerActivity::PeerInputDelivered { message } => json!({
                "type": "peer_message",
                "from": message.from,
                "message": message.message,
            }),
            WorkerActivity::TurnStarted => json!({"type": "turn_start"}),
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
            WorkerActivity::ThinkingStarted { content_index } => {
                self.start_assistant_message();
                json!({
                    "type": "message_update",
                    "assistantMessageEvent": {
                        "type": "thinking_start",
                        "contentIndex": content_index,
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
            WorkerActivity::ToolStarted {
                id,
                name,
                args,
                metadata,
            } => {
                self.finish_assistant_message(None);
                json!({
                    "type": "tool_execution_start",
                    "toolCallId": id,
                    "toolName": name,
                    "args": args,
                    "toolMetadata": metadata,
                })
            }
            WorkerActivity::ChildSessionsChanged {
                id,
                title,
                is_running,
            } => {
                let path = self
                    .path
                    .parent()
                    .and_then(|path| path.parent())
                    .map(|root| external_session_path(root, &self.harness, &id));
                json!({"type": "child_sessions_changed", "child": {
                    "id": id, "path": path, "title": title,
                    "parent_session": self.locator, "is_running": is_running,
                }})
            }
            WorkerActivity::ToolMetadataChanged { id, args, metadata } => {
                let mut event = json!({
                    "type": "tool_metadata_changed",
                    "toolCallId": id,
                    "toolMetadata": metadata,
                });
                if let Some(args) = args {
                    event["args"] = args;
                }
                event
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
                "result": finished_tool_result(result),
                "isError": is_error,
            }),
            WorkerActivity::ToolReviewChanged { id, state, detail } => json!({
                "type": "tool_review_changed",
                "toolCallId": id,
                "state": match state {
                    ToolReviewState::Reviewing => "reviewing",
                    ToolReviewState::Approved => "approved",
                    ToolReviewState::Blocked => "blocked",
                },
                "detail": detail,
            }),
            WorkerActivity::Usage(usage) => {
                self.usage = usage;
                json!({
                    "type": "turn_end",
                    "contextWindow": usage.context_window,
                    "usage": usage_json(usage.turn),
                })
            }
            WorkerActivity::CommandsChanged { commands } => {
                self.metadata.commands.clone_from(&commands);
                self.catalog_response(None, CatalogQuery::Commands);
                return;
            }
            WorkerActivity::TitleChanged(title) => {
                self.metadata.session_name = Some(title);
                self.response(None, Payload::LoadState(Box::new(self.state())));
                return;
            }
            WorkerActivity::ServiceTierChanged { selected, options } => {
                self.metadata.service_tier = selected;
                self.metadata.service_tiers = options;
                self.response(None, Payload::LoadState(Box::new(self.state())));
                return;
            }
            WorkerActivity::ModeChanged(mode) => {
                self.selected_mode = Some(mode);
                self.catalog_response(None, CatalogQuery::Modes);
                return;
            }
            WorkerActivity::ConfigurationChanged {
                models,
                efforts,
                modes,
                selected_model,
                selected_effort,
            } => {
                if let Some(model) = selected_model {
                    self.model = model
                        .get("provider")
                        .and_then(Value::as_str)
                        .zip(model.get("id").and_then(Value::as_str))
                        .map(|(provider, id)| (provider.into(), id.into()));
                    self.usage.context_window = model
                        .get("contextWindow")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                }
                self.effort = selected_effort;
                self.metadata.models.clone_from(&models);
                self.metadata.efforts.clone_from(&efforts);
                self.metadata.modes.clone_from(&modes);
                self.catalog_response(None, CatalogQuery::Models);
                self.response(None, Payload::ListReasoningLevels(efforts));
                self.catalog_response(None, CatalogQuery::Modes);
                self.response(None, Payload::LoadState(Box::new(self.state())));
                return;
            }
            WorkerActivity::ServiceStatusChanged {
                name,
                status,
                error,
                failure_reason,
            } => json!({
                "type": "service_status_changed",
                "name": name,
                "status": status,
                "error": error,
                "failureReason": failure_reason,
            }),
            WorkerActivity::RateLimitsChanged { limits } => json!({
                "type": "rate_limits_changed",
                "limits": limits,
            }),
            WorkerActivity::SessionGoalChanged(goal) => json!({
                "type": "session_goal_changed",
                "goal": goal,
            }),
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

    fn input_delivered(&mut self, mode: WorkerSendMode, text: &str, content: Value) {
        self.acknowledge_delivery(mode, text);
        self.finish_assistant_message(None);
        let message =
            json!({"role":"user", "content":content, "queued":mode != WorkerSendMode::Prompt});
        for event_type in ["message_start", "message_end"] {
            self.pending
                .push_back(activity(json!({"type":event_type, "message":message})));
        }
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
        self.message_count = self.message_count.saturating_add(1);
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
        match command {
            SessionCommand::ConfigureSteering => {
                self.response(Some(id.clone()), Payload::ConfigureSteering)
            }
            SessionCommand::ApplySteering => {
                self.worker.apply_steering()?;
                self.response(Some(id.clone()), Payload::ApplySteering);
            }
            SessionCommand::LoadState => {
                self.response(Some(id.clone()), Payload::LoadState(Box::new(self.state())))
            }
            SessionCommand::LoadHistory => {
                let history = self
                    .history
                    .as_ref()
                    .map_or(SessionHistory::Preserve, |messages| {
                        SessionHistory::Replace(messages.clone())
                    });
                self.response(Some(id.clone()), Payload::LoadHistory(history));
            }
            SessionCommand::LoadUsage => {
                self.response(Some(id.clone()), Payload::LoadUsage(self.session_usage()))
            }
            SessionCommand::ListModels => {
                self.catalog_response(Some(id.clone()), CatalogQuery::Models)
            }
            SessionCommand::ListModes => {
                self.catalog_response(Some(id.clone()), CatalogQuery::Modes)
            }
            SessionCommand::ListCommands => {
                self.catalog_response(Some(id.clone()), CatalogQuery::Commands)
            }
            SessionCommand::ListReasoningLevels => self.response(
                Some(id.clone()),
                Payload::ListReasoningLevels(self.metadata.efforts.clone()),
            ),
            SessionCommand::Prompt {
                mode,
                message,
                images,
            } => {
                let requested_mode = mode;
                // Cursor has no mid-turn steering; show and deliver it as a follow-up.
                let mode = if self.harness == super::cursor::PROFILE.backend
                    && mode == PromptMode::Steer
                {
                    PromptMode::FollowUp
                } else {
                    mode
                };
                let worker_mode = match mode {
                    PromptMode::Normal => WorkerSendMode::Prompt,
                    PromptMode::Steer => WorkerSendMode::Steer,
                    PromptMode::FollowUp => WorkerSendMode::Queue,
                };
                let queued_message = (mode != PromptMode::Normal).then(|| message.clone());
                let accepted =
                    self.worker
                        .submit_prompt(id.clone(), message, worker_mode, images)?;
                self.pending_prompts.insert(
                    id.clone(),
                    PendingPrompt {
                        requested_mode,
                        delivery_mode: mode,
                        queued_message,
                    },
                );
                if accepted {
                    self.finish_prompt_ack(id.clone(), Ok(()));
                }
            }
            SessionCommand::Abort => {
                self.worker.abort()?;
                self.clear_queue();
                self.response(Some(id.clone()), Payload::Abort);
            }
            SessionCommand::SelectModel { provider, model_id } => {
                self.worker.select_model(&provider, &model_id)?;
                self.model = Some((provider.clone(), model_id.clone()));
                self.response(
                    Some(id.clone()),
                    Payload::SelectModel(Model {
                        id: model_id.clone(),
                        name: model_id,
                        provider,
                        context_window: 0,
                        reasoning: true,
                        efforts: None,
                        resolved_model: None,
                        access_modes: None,
                    }),
                );
            }
            SessionCommand::SelectReasoning { level } => {
                self.worker.select_effort(&level)?;
                self.effort = Some(level);
                self.response(Some(id.clone()), Payload::SelectReasoning);
            }
            SessionCommand::SelectServiceTier { tier } => {
                if !self.metadata.service_tiers.contains(&tier) {
                    return Err("Service tier is not available for the selected model".into());
                }
                self.worker.select_service_tier(&tier)?;
                self.metadata.service_tier = Some(tier);
                self.response(Some(id.clone()), Payload::SelectServiceTier);
            }
            SessionCommand::SelectMode { mode } => {
                self.worker.select_mode(&mode)?;
                self.selected_mode = Some(mode);
                self.response(Some(id.clone()), Payload::SelectMode);
            }
            SessionCommand::Compact { .. } => {
                self.worker.compact()?;
                self.response(Some(id.clone()), Payload::Compact);
            }
            SessionCommand::Rename { name } => {
                self.worker.rename(&name)?;
                self.metadata.session_name = Some(name);
                self.response(Some(id.clone()), Payload::Rename);
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
        self.drain_prompt_acks();
        if let Some(event) = self.pending.pop_front() {
            return Some(event);
        }
        let event = self.worker.poll();
        self.drain_prompt_acks();
        if let Some(event) = event {
            self.enqueue_worker_event(event);
        }
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
#[path = "main_session_tests.rs"]
mod tests;
