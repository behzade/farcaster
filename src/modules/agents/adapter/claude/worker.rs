use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
};

use super::super::main_session::{self, MainSessionMetadata};
use super::{
    BACKEND,
    events::{Events, string},
    process::{Process, decode, permission_mode},
};
use crate::agents::{
    AgentLaunchConfig, CallerProfile, CallerRegistry, HarnessAccessMode, PeerMessage,
    SessionLaunch, SessionStart, WorkerActivity, WorkerActivityState, WorkerContext, WorkerEvent,
    WorkerInput, WorkerInputResponse, WorkerLaunch, WorkerSendMode, WorkerSession,
    WorkerSessionFactory,
};
use crate::modules::agents::core::CallerIdentity;
use claude_sdk_types::{
    PermissionResult, Presence, SDKControlInitializeResponse, SDKControlInterruptResponse,
    SDKUserMessage, StdoutMessage,
};
use serde_json::{Value, json};

pub(crate) struct ClaudeWorkerFactory {
    command: AgentLaunchConfig,
}

impl ClaudeWorkerFactory {
    pub(crate) fn new(command: AgentLaunchConfig) -> Self {
        Self { command }
    }
}

impl WorkerSessionFactory for ClaudeWorkerFactory {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        if !matches!(launch.context, WorkerContext::Fresh) {
            return Err("Claude child workers require fresh context".into());
        }
        let caller = CallerRegistry::shared()
            .issue_as(
                &launch.project,
                CallerProfile {
                    backend: BACKEND.into(),
                    provider: launch.provider.clone(),
                    model: launch.model.clone(),
                    effort: launch.effort.clone(),
                },
                None,
                launch.worker_id,
                launch.worker_name,
                launch.parent_worker_id,
            )?
            .with_slot(launch.slot);
        let id = uuid::Uuid::new_v4().to_string();
        // Child sessions use the shared parent/inbox path, never the Farcaster MCP server.
        let process = Process::spawn(
            &self.command,
            &launch.project,
            &id,
            false,
            None,
            None,
            !launch.ephemeral,
        )?;
        let (mut worker, _) = attach(process, caller, &id, self.command.access_mode)?;
        if let Some(model) = launch.model {
            worker.select_model(launch.provider.as_deref().unwrap_or(BACKEND), &model)?;
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

pub(in crate::modules::agents::adapter) fn load_configuration(
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
    )?;
    initialize(&mut process, command.access_mode)
}

pub(in crate::modules::agents::adapter) fn spawn_main(
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
    let caller = CallerRegistry::shared().issue(
        &launch.project,
        CallerProfile {
            backend: BACKEND.into(),
            provider: None,
            model: None,
            effort: None,
        },
        launch.wake.clone(),
    );
    let process = Process::spawn(
        command,
        &launch.project,
        &id,
        resume,
        Some(caller.token()),
        launch.wake.clone(),
        true,
    )?;
    let (worker, metadata) = attach(process, caller, &id, command.access_mode)?;
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
            json!({"id":model.value, "name":model.display_name, "provider":BACKEND,
            "contextWindow":0, "reasoning":value["supportsEffort"].as_bool().unwrap_or(false),
            "resolvedModel":value["resolvedModel"], "efforts":levels,
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
) -> Result<(ClaudeSession, MainSessionMetadata), String> {
    let metadata = initialize(&mut process, access)?;
    caller.bind(id.to_owned());
    let worker = ClaudeSession {
        process,
        caller,
        id: id.into(),
        events: Events::default(),
        active: false,
        active_uuid: None,
        prompt_requests: HashMap::new(),
        prompt_acks: VecDeque::new(),
        closed: false,
        queued: VecDeque::new(),
        permissions: HashMap::new(),
        interrupts: HashSet::new(),
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

struct ClaudeSession {
    process: Process,
    caller: CallerIdentity,
    id: String,
    events: Events,
    active: bool,
    active_uuid: Option<String>,
    prompt_requests: HashMap<String, String>,
    prompt_acks: VecDeque<(String, Result<(), String>)>,
    closed: bool,
    queued: VecDeque<Prompt>,
    permissions: HashMap<String, Value>,
    interrupts: HashSet<String>,
    models: Vec<Value>,
    modes: Vec<Value>,
    model: Option<String>,
    effort: Option<String>,
}

fn prompt(
    id: &str,
    message: &str,
    images: Vec<crate::protocol::PromptImage>,
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
        if mode == WorkerSendMode::Steer {
            return Err("Claude does not support steering".into());
        }
        if self.active {
            self.queued.push_back(prompt);
            Ok(())
        } else {
            self.deliver(prompt)
        }
    }

    fn deliver(&mut self, prompt: Prompt) -> Result<(), String> {
        let active_uuid = match &prompt.message.uuid {
            Presence::Present(uuid) => Some(uuid.clone()),
            Presence::Missing => None,
        };
        self.process.prompt(prompt.message)?;
        self.active_uuid = active_uuid;
        self.active = true;
        self.caller.set_activity(WorkerActivityState::Working);
        self.events.start();
        self.events.activity(prompt.delivery);
        Ok(())
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
            self.caller.select_model(BACKEND, &init.model);
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
            if let Some(ack) = self.prompt_requests.remove(string(&frame, "uuid")) {
                self.prompt_acks.push_back((ack, Ok(())));
            }
        }
        if frame["type"] == "result" && self.active {
            if let Some(ack) = self
                .active_uuid
                .as_ref()
                .and_then(|uuid| self.prompt_requests.remove(uuid))
            {
                let result = if frame["subtype"] == "success"
                    && frame["is_error"] != true
                    && !matches!(
                        string(&frame, "terminal_reason"),
                        "aborted_streaming" | "aborted_tools"
                    ) {
                    Ok(())
                } else {
                    Err("Claude stopped before acknowledging the prompt".into())
                };
                self.prompt_acks.push_back((ack, result));
            }
        }
        match string(&frame, "type") {
            "control_request" => self.control(&frame)?,
            "control_cancel_request" => {
                self.permissions.remove(string(&frame, "request_id"));
            }
            "control_response" => {
                let response = &frame["response"];
                if self.interrupts.remove(string(response, "request_id")) {
                    if response["subtype"] == "error" {
                        return Err(format!("Claude interrupt: {}", string(response, "error")));
                    }
                    // A prompt cancelled before execution has no result frame.
                    // Only settle when the typed receipt names our active prompt.
                    if response["response"].get("cancelled").is_some() {
                        let receipt: SDKControlInterruptResponse =
                            decode(response["response"].clone())?;
                        if let Presence::Present(cancelled) = receipt.cancelled
                            && self
                                .active_uuid
                                .as_ref()
                                .is_some_and(|id| cancelled.contains(id))
                        {
                            if let Some(ack) = self
                                .active_uuid
                                .as_ref()
                                .and_then(|uuid| self.prompt_requests.remove(uuid))
                            {
                                self.prompt_acks.push_back((
                                    ack,
                                    Err("Claude cancelled the prompt before execution".into()),
                                ));
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
                self.idle();
                let interrupted = matches!(
                    string(&frame, "terminal_reason"),
                    "aborted_streaming" | "aborted_tools"
                );
                if !interrupted && (frame["is_error"] == true || frame["subtype"] != "success") {
                    self.queued.clear();
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
    fn send(&mut self, message: String, mode: WorkerSendMode) -> Result<(), String> {
        self.send_with_images(message, mode, Vec::new())
    }
    fn send_with_images(
        &mut self,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<(), String> {
        let images = images
            .into_iter()
            .map(crate::protocol::PromptImage::into_inline)
            .collect::<Result<Vec<_>, _>>()?;
        let prompt = Prompt {
            message: prompt(&self.id, &message, images.clone())?,
            delivery: if images.is_empty() {
                WorkerActivity::InputDelivered { mode, message }
            } else {
                WorkerActivity::InputDeliveredWithImages {
                    mode,
                    message,
                    images,
                }
            },
        };
        self.admit(prompt, mode)
    }
    fn submit_prompt(
        &mut self,
        id: String,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<bool, String> {
        let queued = self.queued.len();
        self.send_with_images(message, mode, images)?;
        let uuid = if self.queued.len() > queued {
            match &self.queued.back().expect("prompt was queued").message.uuid {
                Presence::Present(uuid) => Some(uuid.clone()),
                Presence::Missing => None,
            }
        } else {
            self.active_uuid.clone()
        }
        .ok_or("Claude prompt has no acknowledgement id")?;
        self.prompt_requests.insert(uuid, id);
        Ok(false)
    }

    fn poll_prompt_ack(&mut self) -> Option<(String, Result<(), String>)> {
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
        for prompt in self.queued.drain(..) {
            if let Presence::Present(uuid) = prompt.message.uuid {
                if let Some(ack) = self.prompt_requests.remove(&uuid) {
                    self.prompt_acks.push_back((
                        ack,
                        Err("Claude cancelled a queued prompt before delivery".into()),
                    ));
                }
            }
        }
        if !self.active {
            return Ok(());
        }
        let id = self
            .process
            .request(json!({"subtype":"interrupt", "cancel_queued":true}))?;
        self.interrupts.insert(id);
        Ok(())
    }
    fn poll(&mut self) -> Option<WorkerEvent> {
        if let Some(event) = self.events.pending.pop_front() {
            return Some(event);
        }
        if self.closed {
            return None;
        }
        // Read all already-arrived frames before delivering another turn.
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
            if let Some(prompt) = self.queued.pop_front() {
                if let Err(error) = self.deliver(prompt) {
                    return Some(self.fail(error));
                }
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
        self.events.pending.clear();
        self.process.close()
    }
    fn select_model(&mut self, provider: &str, model: &str) -> Result<(), String> {
        if provider != BACKEND {
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
