use std::{
    collections::{HashMap, VecDeque},
    path::Path,
};

use super::super::main_session::{self, MainSessionMetadata};
use super::super::prompt_batch::PromptBatch;
use super::{
    BACKEND,
    events::{Events, string},
    process::{Process, decode, permission_mode},
};
use crate::core::CallerIdentity;
use crate::{
    AgentLaunchConfig, CallerProfile, CallerRegistry, HarnessAccessMode, PeerMessage,
    SessionLaunch, SessionStart, WorkerActivity, WorkerActivityState, WorkerContext, WorkerEvent,
    WorkerInput, WorkerInputResponse, WorkerLaunch, WorkerSendMode, WorkerSession,
    WorkerSessionFactory,
};
use claude_sdk_types::{
    PermissionResult, Presence, SDKControlInitializeResponse, SDKControlInterruptResponse,
    SDKUserMessage, StdoutMessage,
};
use serde_json::{Value, json};

pub struct ClaudeWorkerFactory {
    command: AgentLaunchConfig,
}

impl ClaudeWorkerFactory {
    pub fn new(command: AgentLaunchConfig) -> Self {
        Self { command }
    }
}

impl WorkerSessionFactory for ClaudeWorkerFactory {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        let (id, resume) = match &launch.context {
            WorkerContext::Fresh => (uuid::Uuid::new_v4().to_string(), false),
            WorkerContext::Session { .. } => {
                return Err("Claude child workers cannot inherit a parent session".into());
            }
            WorkerContext::Resume { session_locator } => (session_locator.clone(), true),
        };
        uuid::Uuid::parse_str(&id).map_err(|_| "Claude requires a UUID session id")?;
        let command = self.command.for_worker(BACKEND, &launch)?;
        let caller = CallerRegistry::shared()
            .issue_as_with_access(
                &launch.project,
                CallerProfile {
                    backend: BACKEND,
                    provider: launch.provider.clone(),
                    model: launch.model.clone(),
                    effort: launch.effort.clone(),
                },
                None,
                launch.worker_id,
                launch.worker_name,
                launch.parent_worker_id,
                launch.access_mode,
            )?
            .with_slot(launch.slot);
        caller.set_harness_profile_id(command.profile_id.clone());
        let process = Process::spawn(
            &command,
            &launch.project,
            &id,
            resume,
            None,
            None,
            !launch.ephemeral,
            launch.service_tier.as_deref(),
        )?;
        let (mut worker, _) = attach(process, caller, &id, launch.access_mode, None)?;
        if let Some(model) = launch.model {
            worker.select_model(
                launch.provider.as_deref().unwrap_or(BACKEND.as_str()),
                &model,
            )?;
        }
        if let Some(effort) = launch.effort {
            worker.select_effort(&effort)?;
        }
        worker
            .events
            .pending
            .push_back(WorkerEvent::SessionChanged { locator: id });
        Ok(Box::new(worker))
    }
}

pub fn load_configuration(
    command: &AgentLaunchConfig,
    project: &Path,
) -> Result<MainSessionMetadata, String> {
    let mut process = Process::spawn(
        command,
        project,
        &uuid::Uuid::new_v4().to_string(),
        false,
        None,
        None,
        false,
        None,
    )?;
    initialize(&mut process, command.access_mode)
}

pub fn spawn_main(
    command: &AgentLaunchConfig,
    launch: &SessionLaunch,
) -> Result<(Box<dyn WorkerSession>, String, MainSessionMetadata), String> {
    let (id, resume) = match &launch.start {
        SessionStart::New => (uuid::Uuid::new_v4().to_string(), false),
        SessionStart::Resume(_) => (
            main_session::launch_session_locator(launch)
                .ok_or("Claude resume requires a session id")?,
            true,
        ),
        SessionStart::Fork(_) => return Err("Claude session fork is not supported".into()),
    };
    uuid::Uuid::parse_str(&id).map_err(|_| "Claude requires a UUID session id")?;
    let caller = CallerRegistry::shared().issue_with_access(
        &launch.project,
        CallerProfile {
            backend: BACKEND,
            provider: None,
            model: None,
            effort: None,
        },
        launch.wake.clone(),
        command.access_mode,
    );
    caller.set_harness_profile_id(command.profile_id.clone());
    let process = Process::spawn(
        command,
        &launch.project,
        &id,
        resume,
        Some(caller.token()),
        launch.wake.clone(),
        true,
        launch.service_tier.as_deref(),
    )?;
    let path = main_session::caller_session_path(command, BACKEND, &id);
    let (worker, mut metadata) = attach(process, caller, &id, command.access_mode, path)?;
    metadata.service_tier = launch.service_tier.clone();
    Ok((Box::new(worker), id, metadata))
}

fn initialize(
    process: &mut Process,
    access: HarnessAccessMode,
) -> Result<MainSessionMetadata, String> {
    let response: SDKControlInitializeResponse =
        decode(process.wait(json!({"subtype":"initialize", "supportedDialogKinds":[]}))?)?;
    let mut efforts = Vec::new();
    let models = response
        .models
        .iter()
        .map(|model| {
            let value = serde_json::to_value(model).expect("SDK model serializes");
            let levels = value["supportedEffortLevels"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for level in &levels {
                if let Some(level) = level.as_str()
                    && !efforts.iter().any(|known| known == level)
                {
                    efforts.push(level.to_owned());
                }
            }
            let resolved = value["resolvedModel"].as_str().unwrap_or(&model.value);
            let service_tiers = if resolved == "opus"
                || resolved.starts_with("claude-opus-5")
                || resolved.starts_with("claude-opus-4-8")
            {
                vec!["standard", "fast"]
            } else {
                Vec::new()
            };
            json!({"id":model.value, "name":model.display_name, "provider":BACKEND,
            "contextWindow":0, "reasoning":value["supportsEffort"].as_bool().unwrap_or(false),
            "resolvedModel":value["resolvedModel"], "efforts":levels,
            "serviceTiers": service_tiers,
            "access_modes": if value["supportsAutoMode"].as_bool() == Some(true) {
                vec![HarnessAccessMode::Sandboxed, HarnessAccessMode::Auto, HarnessAccessMode::Full]
            } else {
                vec![HarnessAccessMode::Sandboxed, HarnessAccessMode::Full]
            }})
        })
        .collect();
    let commands = response
        .commands
        .iter()
        .map(|command| {
            json!({
                "name":command.name, "description":command.description, "source":"prompt",
            })
        })
        .collect();
    let mut modes = vec![
        json!({"id":"default","name":"Ask permissions"}),
        json!({"id":"acceptEdits","name":"Accept edits"}),
    ];
    let name = match access {
        HarnessAccessMode::Auto => Some("Auto"),
        HarnessAccessMode::Full => Some("Full access"),
        HarnessAccessMode::Sandboxed => None,
    };
    if let Some(name) = name {
        modes.insert(0, json!({"id":permission_mode(access),"name":name}));
    }
    Ok(MainSessionMetadata {
        models,
        efforts,
        commands,
        modes,
        ..Default::default()
    })
}

fn attach(
    mut process: Process,
    caller: CallerIdentity,
    id: &str,
    access: HarnessAccessMode,
    locator: Option<std::path::PathBuf>,
) -> Result<(ClaudeSession, MainSessionMetadata), String> {
    let metadata = initialize(&mut process, access)?;
    caller.bind_with_locator(id.to_owned(), locator);
    let worker = ClaudeSession {
        process,
        caller,
        id: id.into(),
        events: Events::default(),
        active: false,
        active_uuid: None,
        dispatched: HashMap::new(),
        prompt_acks: VecDeque::new(),
        closed: false,
        queued: Vec::new(),
        handoff_pending: false,
        handoff_uuid: None,
        abort_waiting_on_handoff_interrupt: None,
        permissions: HashMap::new(),
        interrupts: HashMap::new(),
        models: metadata.models.clone(),
        modes: metadata.modes.clone(),
        model: None,
        effort: None,
    };
    Ok((worker, metadata))
}

struct Prompt {
    message: SDKUserMessage,
    delivery: WorkerActivity,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InterruptPurpose {
    Abort,
    Handoff,
}

struct Interrupt {
    prompt_uuid: String,
    purpose: InterruptPurpose,
}

struct ClaudeSession {
    process: Process,
    caller: CallerIdentity,
    id: String,
    events: Events,
    active: bool,
    active_uuid: Option<String>,
    dispatched: HashMap<String, PromptBatch>,
    prompt_acks: VecDeque<(String, Result<(), crate::PromptRejection>)>,
    closed: bool,
    queued: Vec<Prompt>,
    handoff_pending: bool,
    handoff_uuid: Option<String>,
    abort_waiting_on_handoff_interrupt: Option<String>,
    permissions: HashMap<String, Value>,
    interrupts: HashMap<String, Interrupt>,
    models: Vec<Value>,
    modes: Vec<Value>,
    model: Option<String>,
    effort: Option<String>,
}

fn prompt(
    id: &str,
    message: &str,
    images: Vec<crate::extensions::PromptImage>,
) -> Result<SDKUserMessage, String> {
    let mut content = vec![json!({"type":"text","text":message})];
    for image in images {
        let image = image.into_inline()?;
        content.push(json!({"type":"image", "source":{"type":"base64", "media_type":image.mime_type, "data":image.data}}));
    }
    decode(
        json!({"type":"user", "session_id":id, "uuid":uuid::Uuid::new_v4().to_string(),
        "parent_tool_use_id":null, "message":{"role":"user", "content":content}}),
    )
}

impl ClaudeSession {
    fn prompt(
        &self,
        submission_id: Option<&str>,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::extensions::PromptImage>,
    ) -> Result<Prompt, String> {
        let inline_images = images
            .iter()
            .cloned()
            .map(crate::extensions::PromptImage::into_inline)
            .collect::<Result<Vec<_>, _>>()?;
        let message_frame = prompt(&self.id, &message, inline_images)?;
        let delivery = WorkerActivity::InputDelivered {
            submission_id: submission_id.map(str::to_owned),
            mode,
            message,
            images,
        };
        Ok(Prompt {
            message: message_frame,
            delivery,
        })
    }

    fn idle(&mut self) {
        self.active = false;
        self.active_uuid = None;
        self.permissions.clear();
        self.caller.set_activity(WorkerActivityState::Idle);
    }

    fn admit(&mut self, prompt: Prompt, mode: WorkerSendMode) -> Result<(), String> {
        if self.closed {
            return Err("Claude session is closed".into());
        }
        if self.active || self.handoff_pending || mode == WorkerSendMode::Steer {
            self.queued.push(prompt);
            Ok(())
        } else {
            self.deliver(prompt.message, vec![prompt.delivery]);
            Ok(())
        }
    }

    fn deliver(&mut self, message: SDKUserMessage, deliveries: Vec<WorkerActivity>) {
        let active_uuid = match &message.uuid {
            Presence::Present(uuid) => uuid.clone(),
            Presence::Missing => {
                self.events.pending.push_back(WorkerEvent::Failed(
                    "Claude prompt has no acknowledgement id".into(),
                ));
                return;
            }
        };
        self.active_uuid = Some(active_uuid.clone());
        let deliveries = PromptBatch::from(deliveries);
        self.caller
            .begin_execution(deliveries.first_submission_id());
        self.active = true;
        self.caller.set_activity(WorkerActivityState::Working);
        self.dispatched.insert(active_uuid.clone(), deliveries);
        match self.process.prompt(message) {
            Ok(()) => self.events.start(),
            Err(error) => {
                self.delivery_unknown(
                    &active_uuid,
                    format!("Claude prompt delivery is unknown: {error}"),
                );
                self.events.pending.push_back(WorkerEvent::Failed(error));
            }
        }
    }

    fn delivery_unknown(&mut self, uuid: &str, error: String) {
        let Some(prompt) = self.dispatched.get_mut(uuid) else {
            return;
        };
        prompt.mark_unknown(&error, &mut self.events.pending);
    }

    fn receive_prompt(&mut self, uuid: &str) {
        let Some(mut prompt) = self.dispatched.remove(uuid) else {
            return;
        };
        prompt.acknowledge(&mut self.prompt_acks, &mut self.events.pending);
    }

    fn reject_queued(&mut self, error: &str) {
        PromptBatch::from(
            self.queued
                .drain(..)
                .map(|prompt| prompt.delivery)
                .collect::<Vec<_>>(),
        )
        .reject(error.into(), &mut self.prompt_acks);
    }

    fn interrupt(&mut self, purpose: InterruptPurpose) -> Result<(), String> {
        let Some(prompt_uuid) = self.active_uuid.clone() else {
            return Ok(());
        };
        let id = self
            .process
            .request(json!({"subtype":"interrupt", "cancel_queued":true}))?;
        self.interrupts.insert(
            id,
            Interrupt {
                prompt_uuid,
                purpose,
            },
        );
        Ok(())
    }

    fn handoff_interrupt_pending(&self) -> bool {
        let active_uuid = self.active_uuid.as_deref();
        self.interrupts.values().any(|interrupt| {
            interrupt.purpose == InterruptPurpose::Handoff
                && Some(interrupt.prompt_uuid.as_str()) == active_uuid
        })
    }

    fn dispatch_handoff(&mut self) {
        if !self.handoff_pending || self.active {
            return;
        }
        self.handoff_pending = false;
        self.handoff_uuid = self.dispatch_queued();
    }

    fn dispatch_queued(&mut self) -> Option<String> {
        let mut prompts = std::mem::take(&mut self.queued).into_iter();
        let prompt = prompts.next()?;
        if prompts.len() == 0 {
            let uuid = match &prompt.message.uuid {
                Presence::Present(uuid) => Some(uuid.clone()),
                Presence::Missing => None,
            };
            self.deliver(prompt.message, vec![prompt.delivery]);
            return uuid;
        }
        let prompts = std::iter::once(prompt).chain(prompts);
        let mut content = Vec::new();
        let mut deliveries = Vec::new();
        for (index, prompt) in prompts.enumerate() {
            if index > 0 {
                content.push(json!({"type":"text", "text":"\n\n"}));
            }
            let value =
                serde_json::to_value(prompt.message).expect("Claude SDK user message serializes");
            content.extend(
                value["message"]["content"]
                    .as_array()
                    .expect("Claude prompt content is an array")
                    .iter()
                    .cloned(),
            );
            deliveries.push(prompt.delivery);
        }
        let message: Result<SDKUserMessage, String> = decode(json!({
            "type":"user", "session_id":self.id,
            "uuid":uuid::Uuid::new_v4().to_string(), "parent_tool_use_id":null,
            "message":{"role":"user", "content":content}
        }));
        match message {
            Ok(message) => {
                let uuid = match &message.uuid {
                    Presence::Present(uuid) => Some(uuid.clone()),
                    Presence::Missing => None,
                };
                self.deliver(message, deliveries);
                uuid
            }
            Err(error) => {
                self.events.pending.push_back(WorkerEvent::Failed(error));
                None
            }
        }
    }

    fn reply(&mut self, id: &str, response: impl serde::Serialize) -> Result<(), String> {
        self.process
            .reply(decode(json!({"type":"control_response", "response":{
                "subtype":"success", "request_id":id, "response":response,
            }}))?)
    }

    fn permission(&mut self, id: &str, request: &Value, allow: bool) -> Result<(), String> {
        let result: PermissionResult = decode(if allow {
            json!({"behavior":"allow", "updatedInput":request["input"]})
        } else {
            json!({"behavior":"deny", "message":"Denied by the user or unsupported interactive tool"})
        })?;
        self.reply(id, result)
    }

    fn control(&mut self, frame: &Value) -> Result<(), String> {
        let id = string(frame, "request_id");
        let request = &frame["request"];
        match string(request,"subtype") {
            "can_use_tool" => {
                if request["requires_user_interaction"] == true || request["tool_name"] == "AskUserQuestion" {
                    return self.permission(id, request, false);
                }
                self.permissions.insert(id.into(), request.clone());
                self.events.pending.push_back(WorkerEvent::NeedsInput(WorkerInput {
                    id:id.into(), prompt:format!("Allow {}?\n{}", string(request,"tool_name"), request["input"]),
                    options:vec!["Deny".into(), "Allow".into()], secret:false,
                }));
                Ok(())
            }
            _ => self.process.reply(decode(json!({"type":"control_response", "response":{
                "subtype":"error", "request_id":id,
                "error":format!("Farcaster does not support Claude control request {}", string(request,"subtype")),
            }}))?),
        }
    }

    fn receive(&mut self, frame: StdoutMessage) -> Result<(), String> {
        if let StdoutMessage::SDKSystemMessage(init) = &frame {
            self.model = Some(init.model.clone());
            self.caller.select_model(BACKEND.as_str(), &init.model);
            if let claude_sdk_types::Presence::Present(effort) = &init.effort {
                self.effort = effort.as_ref().map(|effort| {
                    serde_json::to_value(effort)
                        .expect("SDK effort serializes")
                        .as_str()
                        .unwrap_or_default()
                        .to_owned()
                });
            }
            self.configuration_changed();
        }
        let frame = serde_json::to_value(frame).map_err(|error| error.to_string())?;
        if frame["type"] == "system"
            && frame["subtype"] == "init"
            && frame["session_id"].as_str().is_some_and(|id| id != self.id)
        {
            return Err("Claude initialized a different session than requested".into());
        }
        if frame["type"] == "user" && frame["session_id"].as_str() == Some(self.id.as_str()) {
            self.receive_prompt(string(&frame, "uuid"));
        }
        match string(&frame, "type") {
            "control_request" => self.control(&frame)?,
            "control_cancel_request" => {
                self.permissions.remove(string(&frame, "request_id"));
            }
            "control_response" => {
                let response = &frame["response"];
                if let Some(interrupt) = self.interrupts.remove(string(response, "request_id")) {
                    if response["subtype"] == "error" {
                        if self.abort_waiting_on_handoff_interrupt.as_deref()
                            == Some(&interrupt.prompt_uuid)
                            && self.active_uuid.as_deref() == Some(&interrupt.prompt_uuid)
                        {
                            self.abort_waiting_on_handoff_interrupt = None;
                            self.interrupt(InterruptPurpose::Abort)?;
                            return Ok(());
                        }
                        if self.active_uuid.as_deref() == Some(&interrupt.prompt_uuid) {
                            self.events.pending.push_back(WorkerEvent::RequestFailed {
                                operation: "interrupt".into(),
                                error: format!("Claude interrupt: {}", string(response, "error")),
                            });
                        }
                        return Ok(());
                    }
                    if self.abort_waiting_on_handoff_interrupt.as_deref()
                        == Some(&interrupt.prompt_uuid)
                    {
                        self.abort_waiting_on_handoff_interrupt = None;
                    }
                    if response["response"].get("cancelled").is_some() {
                        let receipt: SDKControlInterruptResponse =
                            decode(response["response"].clone())?;
                        if let Presence::Present(cancelled) = receipt.cancelled
                            && cancelled.contains(&interrupt.prompt_uuid)
                            && self.active_uuid.as_deref() == Some(&interrupt.prompt_uuid)
                        {
                            self.delivery_unknown(
                                &interrupt.prompt_uuid,
                                "Claude cancelled a dispatched prompt before receipt".into(),
                            );
                            if interrupt.purpose == InterruptPurpose::Abort
                                && self.handoff_uuid.as_deref() == Some(&interrupt.prompt_uuid)
                            {
                                self.handoff_uuid = None;
                            }
                            self.idle();
                            self.events.pending.push_back(WorkerEvent::Settled {
                                output: String::new(),
                            });
                        }
                    }
                }
            }
            "result" if self.active => {
                self.events.message(&frame);
                if let Some(uuid) = self.active_uuid.clone() {
                    if self.abort_waiting_on_handoff_interrupt.as_deref() == Some(&uuid) {
                        self.abort_waiting_on_handoff_interrupt = None;
                    }
                    self.delivery_unknown(
                        &uuid,
                        "Claude finished before confirming prompt receipt".into(),
                    );
                    if self.handoff_uuid.as_deref() == Some(&uuid) {
                        self.handoff_uuid = None;
                    }
                }
                self.idle();
                let interrupted = matches!(
                    string(&frame, "terminal_reason"),
                    "aborted_streaming" | "aborted_tools"
                );
                if !interrupted && (frame["is_error"] == true || frame["subtype"] != "success") {
                    self.handoff_pending = false;
                    self.reject_queued("Claude cancelled a queued prompt after execution failed");
                    self.events.pending.push_back(WorkerEvent::Failed(
                        frame["errors"]
                            .as_array()
                            .map(|errors| {
                                errors
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                            .filter(|error| !error.is_empty())
                            .unwrap_or_else(|| {
                                let result = string(&frame, "result");
                                if result.is_empty() {
                                    format!("Claude result: {}", string(&frame, "subtype"))
                                } else {
                                    result.into()
                                }
                            }),
                    ));
                } else {
                    let result = frame["result"]
                        .as_str()
                        .filter(|result| !result.is_empty())
                        .unwrap_or(&self.events.output)
                        .to_owned();
                    self.events
                        .pending
                        .push_back(WorkerEvent::Settled { output: result });
                }
            }
            _ => self.events.message(&frame),
        }
        Ok(())
    }

    fn fail(&mut self, error: String) -> WorkerEvent {
        let _ = self.close();
        WorkerEvent::Failed(error)
    }

    fn selected_model(&self) -> Option<&Value> {
        match self.model.as_deref() {
            Some(id) => self
                .models
                .iter()
                .find(|model| model["id"] == id || model["resolvedModel"] == id),
            None => self.models.first(),
        }
    }

    fn configuration_changed(&mut self) {
        let efforts = self
            .selected_model()
            .and_then(|model| model["efforts"].as_array())
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let selected_model = self.model.as_ref().map(|id| {
            let mut model = self
                .selected_model()
                .cloned()
                .unwrap_or_else(|| json!({"id":id,"name":id,"provider":BACKEND,"contextWindow":0}));
            model["id"] = json!(id);
            model
        });
        self.events.activity(WorkerActivity::ConfigurationChanged {
            models: self.models.clone(),
            efforts,
            modes: self.modes.clone(),
            selected_model,
            selected_effort: self.effort.clone(),
        });
    }
}

impl WorkerSession for ClaudeSession {
    fn has_exited(&mut self) -> bool {
        self.process.has_exited()
    }

    fn retain_inbox(&self) -> Result<Option<Box<dyn crate::SessionInbox>>, String> {
        self.caller.retain_inbox()
    }

    fn can_retire(&self) -> bool {
        !self.active
            && !self.closed
            && self.dispatched.is_empty()
            && self.queued.is_empty()
            && !self.handoff_pending
            && self.handoff_uuid.is_none()
            && self.abort_waiting_on_handoff_interrupt.is_none()
            && self.permissions.is_empty()
            && self.interrupts.is_empty()
            && self.prompt_acks.is_empty()
            && self.events.pending.is_empty()
            && !self.caller.has_pending_messages()
    }

    fn tracks_prompt_delivery(&self, _mode: WorkerSendMode) -> bool {
        true
    }

    fn send(&mut self, message: String, mode: WorkerSendMode) -> Result<(), String> {
        self.send_with_images(message, mode, Vec::new())
    }
    fn send_with_images(
        &mut self,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::extensions::PromptImage>,
    ) -> Result<(), String> {
        let prompt = self.prompt(None, message, mode, images)?;
        self.admit(prompt, mode)
    }
    fn submit_prompt(
        &mut self,
        id: String,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::extensions::PromptImage>,
    ) -> Result<bool, crate::PromptRejection> {
        let prompt = self.prompt(Some(&id), message, mode, images)?;
        self.admit(prompt, mode)?;
        Ok(false)
    }

    fn poll_prompt_ack(&mut self) -> Option<(String, Result<(), crate::PromptRejection>)> {
        self.prompt_acks.pop_front()
    }

    fn send_peer_message(
        &mut self,
        message: &PeerMessage,
        mode: WorkerSendMode,
    ) -> Result<(), String> {
        self.admit(
            Prompt {
                message: prompt(&self.id, &message.prompt(), Vec::new())?,
                delivery: WorkerActivity::PeerInputDelivered {
                    message: message.clone(),
                },
            },
            mode,
        )
    }
    fn respond(&mut self, response: WorkerInputResponse) -> Result<(), String> {
        let request = self
            .permissions
            .get(&response.id)
            .ok_or("Claude permission request expired")?
            .clone();
        self.permission(
            &response.id,
            &request,
            !response.cancel && response.value.as_deref() == Some("Allow"),
        )?;
        self.permissions.remove(&response.id);
        Ok(())
    }
    fn abort(&mut self) -> Result<(), String> {
        let pending_handoff = self.handoff_pending;
        self.handoff_pending = false;
        self.reject_queued("Claude cancelled a queued prompt before dispatch");
        if !self.active {
            return Ok(());
        }
        if pending_handoff && self.handoff_uuid.is_none() && self.handoff_interrupt_pending() {
            self.abort_waiting_on_handoff_interrupt = self.active_uuid.clone();
            return Ok(());
        }
        self.interrupt(InterruptPurpose::Abort)
    }
    fn apply_steering(&mut self) -> Result<(), String> {
        if self.closed {
            return Err("Claude session is closed".into());
        }
        if self.queued.is_empty() || self.handoff_pending {
            return Ok(());
        }
        self.handoff_pending = true;
        if self.active {
            self.interrupt(InterruptPurpose::Handoff)
        } else {
            self.dispatch_handoff();
            Ok(())
        }
    }
    fn poll(&mut self) -> Option<WorkerEvent> {
        if let Some(event) = self.events.pending.pop_front() {
            return Some(event);
        }
        if self.closed {
            return None;
        }
        for _ in 0..128 {
            let Some(frame) = self.process.poll() else {
                break;
            };
            if let Err(error) = frame.and_then(|frame| self.receive(frame)) {
                return Some(self.fail(error));
            }
            if let Some(event) = self.events.pending.pop_front() {
                return Some(event);
            }
        }
        if !self.active {
            if self.handoff_pending {
                self.dispatch_handoff();
            } else if !self.queued.is_empty() {
                self.dispatch_queued();
            } else if let Some(message) = self.caller.try_recv()
                && let Err(error) = self.send_peer_message(&message, WorkerSendMode::Prompt)
            {
                return Some(self.fail(error));
            }
        }
        self.events.pending.pop_front()
    }
    fn close(&mut self) -> Result<(), String> {
        self.closed = true;
        self.idle();
        self.queued.clear();
        self.dispatched.clear();
        self.handoff_pending = false;
        self.handoff_uuid = None;
        self.abort_waiting_on_handoff_interrupt = None;
        self.events.pending.clear();
        self.process.close()
    }
    fn select_model(&mut self, provider: &str, model: &str) -> Result<(), String> {
        if provider != BACKEND.as_str() {
            return Err(format!("Claude does not support provider {provider}"));
        }
        self.process
            .wait(json!({"subtype":"set_model", "model":model}))?;
        self.model = Some(model.into());
        self.caller.select_model(provider, model);
        self.configuration_changed();
        Ok(())
    }
    fn select_effort(&mut self, effort: &str) -> Result<(), String> {
        let selected = self.selected_model();
        if !selected
            .and_then(|model| model["efforts"].as_array())
            .is_some_and(|levels| levels.iter().any(|level| level == effort))
        {
            return Err(format!("Claude model does not advertise effort {effort}"));
        }
        self.process
            .wait(json!({"subtype":"apply_flag_settings", "settings":{"effortLevel":effort}}))?;
        self.caller.select_effort(effort);
        self.effort = Some(effort.into());
        Ok(())
    }
    fn model_selection(&self) -> Option<crate::WorkerModelSelection> {
        Some(crate::WorkerModelSelection {
            model: self.model.clone().map(|model| (BACKEND.into(), model)),
            effort: self.effort.clone(),
        })
    }
    fn select_mode(&mut self, mode: &str) -> Result<(), String> {
        if !self.modes.iter().any(|entry| entry["id"] == mode) {
            return Err(format!("Claude permission mode {mode} is not available"));
        }
        self.process
            .wait(json!({"subtype":"set_permission_mode", "mode":mode}))?;
        self.events
            .activity(WorkerActivity::ModeChanged(mode.into()));
        Ok(())
    }
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
