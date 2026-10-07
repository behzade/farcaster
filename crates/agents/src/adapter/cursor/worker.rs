use super::{
    bridge::Bridge,
    configuration::{self, Model},
    events::Events,
};
use crate::adapter::{farcaster_mcp, main_session, prompt_input::PromptInput};
use crate::{
    AgentLaunchConfig, Backend, HarnessAccessMode, WorkerActivity, WorkerActivityState,
    WorkerEvent, WorkerInputResponse, WorkerLaunch, WorkerModelSelection, WorkerSendMode,
    WorkerSession, WorkerSessionFactory, WorkerUsage,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

type Reply = mpsc::Receiver<Result<Value, String>>;

const STEER_FOLLOW_UP: &str = "Cursor SDK steering reverted to follow-up";

pub(super) struct Factory(pub(super) AgentLaunchConfig);

impl WorkerSessionFactory for Factory {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        if launch.ephemeral && !matches!(launch.context, crate::WorkerContext::Fresh) {
            return Err("Cursor temporary workers require fresh context".into());
        }
        if launch.provider.is_some() != launch.model.is_some()
            || launch
                .provider
                .as_deref()
                .is_some_and(|p| p != Backend::Cursor.as_str())
        {
            return Err("Cursor SDK worker requires a Cursor provider/model pair".into());
        }
        let config = self.0.for_worker(Backend::Cursor, &launch)?;
        let identity = if launch.ephemeral {
            None
        } else {
            let identity = crate::core::CallerRegistry::shared()
                .issue_as_with_access(
                    &launch.project,
                    crate::core::CallerProfile {
                        backend: Backend::Cursor,
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
            identity.set_harness_profile_id(config.profile_id.clone());
            Some(identity)
        };
        let resume = match &launch.context {
            crate::WorkerContext::Fresh => None,
            crate::WorkerContext::Resume { session_locator } => Some(session_locator.as_str()),
            crate::WorkerContext::Session { .. } => {
                return Err("Cursor SDK session fork is not supported".into());
            }
        };
        let bridge = Bridge::start_live(&config, &launch.project, launch.ephemeral)?;
        let (mut worker, _) =
            Worker::from_bridge(bridge, &config, &launch.project, resume, None, None)?;
        if let Some(model) = launch.model {
            worker.select_model(Backend::Cursor.as_str(), &model)?;
        }
        if let Some(effort) = launch.effort {
            worker.select_effort(&effort)?;
        }
        if let Some(tier) = launch.service_tier {
            worker.select_service_tier(&tier)?;
        }
        if let Some(identity) = &identity {
            identity.bind(worker.id.clone());
            worker.events.push_back(WorkerEvent::SessionChanged {
                locator: worker.id.clone(),
            });
        }
        worker.identity = identity;
        Ok(Box::new(worker))
    }
}

type MainSession = (
    Box<dyn WorkerSession>,
    String,
    main_session::MainSessionMetadata,
    Option<crate::DiscoveredHistory>,
);

pub(super) fn spawn_main(
    config: &AgentLaunchConfig,
    launch: &crate::SessionLaunch,
) -> Result<MainSession, String> {
    let identity = crate::core::CallerRegistry::shared().issue_with_access(
        &launch.project,
        crate::core::CallerProfile {
            backend: Backend::Cursor,
            provider: None,
            model: None,
            effort: None,
        },
        launch.wake.clone(),
        config.access_mode,
    );
    identity.set_harness_profile_id(config.profile_id.clone());
    let resume = match &launch.start {
        crate::SessionStart::New => None,
        crate::SessionStart::Resume(_) => Some(
            main_session::launch_session_locator(launch).ok_or("Cursor SDK session id missing")?,
        ),
        crate::SessionStart::Fork(_) => {
            return Err("Cursor SDK session fork is not supported".into());
        }
    };
    let bridge = Bridge::start_live(config, &launch.project, false)?;
    let (mut worker, metadata) = Worker::from_bridge(
        bridge,
        config,
        &launch.project,
        resume.as_deref(),
        Some(identity.token()),
        launch.wake.clone(),
    )?;
    let history = resume
        .as_deref()
        .map(|id| super::history::load(&worker.bridge, id, &launch.project))
        .transpose()?;
    if let Some(tier) = launch.service_tier.as_deref() {
        worker.select_service_tier(tier)?;
    }
    let id = worker.id.clone();
    identity.bind_with_locator(
        id.clone(),
        config
            .locator_root()
            .map(|root| main_session::external_session_path(&root, Backend::Cursor, &id)),
    );
    identity.select_model(Backend::Cursor.as_str(), &worker.model.id);
    worker.identity = Some(identity);
    Ok((Box::new(worker), id, metadata, history))
}

pub(super) struct Worker {
    bridge: Bridge,
    id: String,
    models: Vec<Model>,
    model: Model,
    mode: String,
    identity: Option<crate::core::CallerIdentity>,
    wake: Option<thread::Thread>,
    stream: Option<mpsc::Receiver<Result<Option<Value>, String>>>,
    current: Option<PromptInput>,
    queue: VecDeque<PromptInput>,
    peers: HashMap<String, crate::PeerMessage>,
    steering: Vec<(PromptInput, Reply)>,
    prompt_acks: VecDeque<(String, Result<(), crate::PromptRejection>)>,
    pending_finish: Option<Option<String>>,
    metadata_request: Option<Reply>,
    metadata_dirty: bool,
    events: VecDeque<WorkerEvent>,
    run_id: Option<String>,
    terminal: Option<Value>,
    last_error: Option<String>,
    cancelled: Option<Instant>,
    cancel_sent: bool,
    translated: Events,
    session_usage: crate::TokenUsage,
}

impl Worker {
    pub(super) fn from_bridge(
        bridge: Bridge,
        config: &AgentLaunchConfig,
        project: &Path,
        resume: Option<&str>,
        caller: Option<&str>,
        wake: Option<thread::Thread>,
    ) -> Result<(Self, main_session::MainSessionMetadata), String> {
        let (catalog, saved) = thread::scope(|scope| {
            let catalog = scope.spawn(|| {
                super::super::catalog_cache::load(config, Backend::Cursor, project, || {
                    configuration::catalog(&bridge.models()?)
                })
            });
            let saved = resume.map(|id| bridge.agent(
                "ListRuns", json!({"agentId":id,"options":{"runtime":"RUNTIME_LOCAL","cwd":project,"limit":1}})
            )).transpose();
            (catalog.join().expect("Cursor model lookup thread"), saved)
        });
        let models =
            configuration::from_catalog(&catalog?).ok_or("Cursor catalog lacks request data")?;
        let mut model = models
            .first()
            .cloned()
            .ok_or("Cursor SDK returned no usable models")?;
        if let Some(saved) = saved?
            .as_ref()
            .and_then(|runs| runs.pointer("/items/0/model"))
        {
            model = configuration::restore(&models, saved)?;
        }
        let mut options = json!({"model":model.wire,"local":{
            "cwd":[project],"settingSources":["SETTING_SOURCE_PROJECT","SETTING_SOURCE_USER"],
            "sandboxOptions":{"enabled":config.access_mode != HarnessAccessMode::Full},
            "autoReview":config.access_mode == HarnessAccessMode::Auto
        }});
        if let Some(key) = bridge.api_key() {
            options["apiKey"] = key.into();
        }
        if farcaster_mcp::enabled()
            && let Some(token) = caller
        {
            options["mcpServers"] = json!({"farcaster":{"http":{"type":"HTTP_MCP_TRANSPORT_TYPE_HTTP","url":farcaster_mcp::url(),"headers":{(farcaster_mcp::CALLER_HEADER):token}}}});
        }
        let response = match resume {
            Some(id) => bridge.agent("ResumeAgent", json!({"agentId":id,"options":options}))?,
            None => bridge.agent("CreateAgent", json!({"options":options}))?,
        };
        let id = response["agentId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Cursor SDK omitted agentId")?
            .to_owned();
        let metadata = configuration::metadata(&models, &model);
        let metadata_dirty = !bridge.is_ephemeral();
        let mut worker = Self {
            bridge,
            id,
            models,
            model,
            mode: "agent".into(),
            identity: None,
            wake,
            stream: None,
            current: None,
            queue: VecDeque::new(),
            peers: HashMap::new(),
            steering: Vec::new(),
            prompt_acks: VecDeque::new(),
            pending_finish: None,
            metadata_request: None,
            metadata_dirty,
            events: VecDeque::new(),
            run_id: None,
            terminal: None,
            last_error: None,
            cancelled: None,
            cancel_sent: false,
            translated: Events::default(),
            session_usage: Default::default(),
        };
        worker.poll_metadata();
        Ok((worker, metadata))
    }

    fn request(&self, method: &'static str, body: Value) -> Result<Reply, String> {
        let client = self.bridge.client.clone();
        let wake = self.wake.clone();
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name(format!("cursor-sdk-{method}"))
            .spawn(move || {
                let _ = sender.send(client.call("SdkAgentService", method, body));
                if let Some(wake) = wake {
                    wake.unpark();
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(receiver)
    }

    fn poll_metadata(&mut self) {
        if let Some(request) = &self.metadata_request {
            match request.try_recv() {
                Ok(result) => {
                    self.metadata_request = None;
                    if let Ok(info) = result
                        && let Some(name) = info
                            .pointer("/agent/name")
                            .and_then(Value::as_str)
                            .filter(|name| !name.trim().is_empty() && name.trim() != "New Agent")
                    {
                        self.events
                            .push_back(WorkerEvent::Activity(WorkerActivity::TitleChanged(
                                name.into(),
                            )));
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => self.metadata_request = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.metadata_request.is_none() && self.metadata_dirty {
            self.metadata_dirty = false;
            self.metadata_request = self.request("GetAgent", json!({"agentId":self.id})).ok();
        }
    }

    fn admit(&mut self, mut input: PromptInput) -> Result<(), String> {
        if self.bridge.has_exited() {
            return Err("Cursor SDK bridge has exited".into());
        }
        input.images = input
            .images
            .into_iter()
            .map(crate::extensions::PromptImage::into_inline)
            .collect::<Result<_, _>>()?;
        if input.mode == WorkerSendMode::Steer {
            return self.steer(input);
        }
        if self.stream.is_some() || self.pending_finish.is_some() {
            if input.mode != WorkerSendMode::Queue {
                return Err("Cursor SDK agent is already working".into());
            }
            self.queue.push_back(input);
            return Ok(());
        }
        self.start_turn(input)
    }

    fn steer(&mut self, input: PromptInput) -> Result<(), String> {
        if !input.images.is_empty() || self.stream.is_none() {
            if let Some(id) = &input.submission_id {
                self.prompt_acks
                    .push_back((id.clone(), Err(STEER_FOLLOW_UP.into())));
            }
            return Ok(());
        }
        let receiver = self.request(
            "SteerRun",
            json!({
                "agentId":self.id,"runId":self.run_id,"text":input.message
            }),
        )?;
        self.steering.push((input, receiver));
        Ok(())
    }

    fn poll_steering(&mut self) {
        let mut index = 0;
        while index < self.steering.len() {
            let result = match self.steering[index].1.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => {
                    index += 1;
                    continue;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    Err("Cursor steering connection closed".into())
                }
            };
            let (input, _) = self.steering.remove(index);
            let id = input.submission_id.clone();
            match result {
                Ok(value) if value["outcome"] == "complete_delivered" => {
                    self.delivered();
                    if let Some(id) = id {
                        self.prompt_acks.push_back((id, Ok(())));
                    }
                    self.emit_delivery(input);
                }
                Ok(value) if value["outcome"] == "revert_to_followup" => {
                    if let Some(id) = id {
                        self.prompt_acks
                            .push_back((id, Err(STEER_FOLLOW_UP.into())));
                    }
                }
                result => {
                    if let Some(submission_id) = id {
                        let error = result.err().unwrap_or_else(|| {
                            "Cursor returned an unknown steering outcome".into()
                        });
                        self.events.push_back(WorkerEvent::PromptDeliveryUnknown {
                            submission_id,
                            error,
                        });
                    }
                }
            }
        }
    }

    fn start_turn(&mut self, input: PromptInput) -> Result<(), String> {
        let images: Vec<_> = input
            .images
            .iter()
            .map(|i| json!({"data":{"data":i.data,"mimeType":i.mime_type}}))
            .collect();
        let body = json!({"agentId":self.id,"message":{"text":input.message,"images":images},"options":{"model":self.model.wire,"mode":if self.mode == "plan" {"AGENT_MODE_OPTION_PLAN"} else {"AGENT_MODE_OPTION_AGENT"},"enableDeltas":true}});
        let client = self.bridge.client.clone();
        let wake = self.wake.clone();
        let (tx, rx) = mpsc::sync_channel(256);
        thread::Builder::new()
            .name("cursor-sdk-run".into())
            .spawn(move || {
                let result = (|| {
                    let mut stream = client.send(body)?;
                    while let Some(value) = stream.next()? {
                        if tx.send(Ok(Some(value))).is_err() {
                            return Ok(());
                        }
                        if let Some(wake) = &wake {
                            wake.unpark();
                        }
                    }
                    Ok(())
                })();
                let _ = tx.send(result.map(|()| None));
                if let Some(wake) = wake {
                    wake.unpark();
                }
            })
            .map_err(|e| format!("Start Cursor SDK stream: {e}"))?;
        if let Some(identity) = &self.identity {
            identity.begin_execution(input.submission_id.as_deref());
            identity.set_activity(WorkerActivityState::Working);
        }
        self.stream = Some(rx);
        self.current = Some(input);
        self.run_id = None;
        self.terminal = None;
        self.last_error = None;
        self.cancelled = None;
        self.cancel_sent = false;
        self.translated = Events::default();
        self.events.push_back(WorkerEvent::Started);
        Ok(())
    }

    fn delivered(&mut self) {
        if let Some(input) = self.current.take() {
            self.emit_delivery(input);
        }
    }

    fn emit_delivery(&mut self, input: PromptInput) {
        let peer = input
            .submission_id
            .as_ref()
            .and_then(|id| self.peers.remove(id));
        let activity = peer.map_or_else(
            || input.into_activity(),
            |message| WorkerActivity::PeerInputDelivered { message },
        );
        self.events.push_back(WorkerEvent::Activity(activity));
    }

    fn cancel_run(&mut self) -> Result<(), String> {
        if self.cancelled.is_some()
            && !self.cancel_sent
            && let Some(id) = &self.run_id
        {
            self.bridge
                .agent("CancelRun", json!({"agentId":self.id,"runId":id}))?;
            self.cancel_sent = true;
        }
        Ok(())
    }

    fn envelope(&mut self, value: Value) -> Result<(), String> {
        if let Some(id) = value.pointer("/runStarted/runId").and_then(Value::as_str) {
            self.run_id = Some(id.into());
        }
        if let Some(update) = value.get("interactionUpdate") {
            if matches!(
                update["type"].as_str(),
                Some("text-delta" | "thinking-delta" | "tool-call-started")
            ) {
                self.delivered();
            }
            for activity in self.translated.interaction(update) {
                self.events.push_back(WorkerEvent::Activity(activity));
            }
        }
        // The helper forwards SDK control messages only; activity uses onDelta.
        if let Some(message) = value.get("sdkMessage") {
            let payload = &message["message"];
            if self.run_id.is_none() {
                self.run_id = payload["run_id"].as_str().map(str::to_owned);
            }
            let kind = message["type"].as_str().unwrap_or_default();
            if matches!(kind, "assistant" | "thinking" | "tool_call") {
                self.delivered();
            }
            if kind == "status" {
                self.last_error = payload["message"].as_str().map(str::to_owned);
            }
            for activity in self.translated.message(kind, payload) {
                self.events.push_back(WorkerEvent::Activity(activity));
            }
        }
        if let Some(result) = value.get("result") {
            if self.run_id.is_none() {
                self.run_id = result["runId"].as_str().map(str::to_owned);
            }
            self.terminal = Some(result.clone());
        }
        self.cancel_run()
    }

    fn finish(&mut self, transport_error: Option<String>) {
        self.stream = None;
        if !self.steering.is_empty() {
            self.pending_finish = Some(transport_error);
            return;
        }
        if let Some(identity) = &self.identity {
            identity.set_activity(WorkerActivityState::Idle);
        }
        let terminal = self.terminal.take();
        let status = terminal
            .as_ref()
            .and_then(|v| v["status"].as_str())
            .unwrap_or_default();
        let success = transport_error.is_none()
            && matches!(
                status,
                "RUN_LIFECYCLE_STATUS_FINISHED" | "RUN_LIFECYCLE_STATUS_CANCELLED"
            );
        if status == "RUN_LIFECYCLE_STATUS_FINISHED" && transport_error.is_none() {
            self.delivered();
        }
        if let Some(input) = self.current.take()
            && let Some(id) = input.submission_id
        {
            self.events.push_back(WorkerEvent::PromptDeliveryUnknown {
                submission_id: id,
                error: "Cursor SDK ended without evidence of model delivery".into(),
            });
        }
        if success {
            if let Some(usage) = terminal.as_ref().and_then(|v| v.pointer("/result/usage")) {
                let turn = super::events::usage(usage);
                self.session_usage = self.session_usage.saturating_add(turn);
                self.events
                    .push_back(WorkerEvent::Activity(WorkerActivity::Usage(WorkerUsage {
                        turn,
                        session: self.session_usage,
                        ..Default::default()
                    })));
            }
            let output = terminal
                .as_ref()
                .and_then(|v| v.pointer("/result/result"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(&self.translated.output)
                .to_owned();
            self.metadata_dirty = !self.bridge.is_ephemeral();
            self.events.push_back(WorkerEvent::Settled { output });
        } else {
            let error = transport_error
                .or_else(|| self.last_error.take())
                .or_else(|| {
                    terminal
                        .as_ref()
                        .and_then(|v| v["errorCode"].as_str())
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| {
                    format!("Cursor SDK run did not finish successfully ({status})")
                });
            self.metadata_request = None;
            self.metadata_dirty = false;
            // A disconnected Send does not cancel the SDK run.
            self.cancelled = Some(Instant::now());
            let _ = self.cancel_run();
            let _ = self.bridge.close();
            self.cancel_queue();
            self.events.push_back(WorkerEvent::Failed(error));
        }
        self.peers.retain(|id, _| {
            self.queue
                .iter()
                .any(|input| input.submission_id.as_ref() == Some(id))
        });
        self.cancelled = None;
    }

    fn cancel_queue(&mut self) {
        for input in self.queue.drain(..) {
            if let Some(id) = input.submission_id {
                if self.peers.remove(&id).is_some() {
                    continue;
                }
                self.events
                    .push_back(WorkerEvent::PromptCancelled { submission_id: id });
            }
        }
    }

    fn selection_changed(&mut self) {
        if let Some(identity) = &self.identity {
            identity.select_model(Backend::Cursor.as_str(), &self.model.id);
            identity.set_effort(self.model.effort().as_deref());
        }
        self.events.push_back(WorkerEvent::Activity(
            WorkerActivity::ConfigurationChanged {
                models: self.models.iter().map(|m| m.display.clone()).collect(),
                efforts: self.model.efforts.clone(),
                modes: configuration::metadata(&self.models, &self.model).modes,
                selected_model: Some(self.model.display.clone()),
                selected_effort: self.model.effort(),
            },
        ));
        self.events
            .push_back(WorkerEvent::Activity(WorkerActivity::ServiceTierChanged {
                selected: self.model.tier(),
                options: self.model.tiers.clone(),
            }));
    }
}

impl WorkerSession for Worker {
    fn rename(&mut self, name: &str) -> Result<(), String> {
        self.bridge
            .agent("RenameAgent", json!({"agentId":self.id,"name":name}))?;
        // Discard a lookup started before the rename; its old name is now stale.
        self.metadata_request = None;
        self.metadata_dirty = false;
        self.events.retain(|event| {
            !matches!(
                event,
                WorkerEvent::Activity(WorkerActivity::TitleChanged(_))
            )
        });
        Ok(())
    }
    fn has_exited(&mut self) -> bool {
        self.bridge.has_exited()
    }
    fn retain_inbox(&self) -> Result<Option<Box<dyn crate::SessionInbox>>, String> {
        self.identity
            .as_ref()
            .map_or(Ok(None), |caller| caller.retain_inbox())
    }
    fn send_peer_message(
        &mut self,
        message: &crate::PeerMessage,
        mode: WorkerSendMode,
    ) -> Result<(), String> {
        let id = format!("cursor-peer-{}", uuid::Uuid::new_v4());
        self.admit(PromptInput {
            submission_id: Some(id.clone()),
            message: message.prompt(),
            mode,
            images: Vec::new(),
        })?;
        self.peers.insert(id, message.clone());
        Ok(())
    }
    fn can_retire(&self) -> bool {
        self.stream.is_none()
            && self.pending_finish.is_none()
            && self.steering.is_empty()
            && self.queue.is_empty()
            && self.events.is_empty()
            && self.prompt_acks.is_empty()
            && self
                .identity
                .as_ref()
                .is_none_or(|i| !i.has_pending_messages())
    }
    fn steer_error_recovery(&self, error: &str) -> crate::SteerErrorRecovery {
        if error == STEER_FOLLOW_UP {
            crate::SteerErrorRecovery::RetryWhenIdle
        } else {
            crate::SteerErrorRecovery::Fail
        }
    }
    fn poll_prompt_ack(&mut self) -> Option<(String, Result<(), crate::PromptRejection>)> {
        self.poll_steering();
        self.prompt_acks.pop_front()
    }
    fn tracks_prompt_delivery(&self, _: WorkerSendMode) -> bool {
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
        self.admit(PromptInput {
            submission_id: None,
            message,
            mode,
            images,
        })
    }
    fn submit_prompt(
        &mut self,
        id: String,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::extensions::PromptImage>,
    ) -> Result<bool, crate::PromptRejection> {
        self.admit(PromptInput {
            submission_id: Some(id),
            message,
            mode,
            images,
        })?;
        Ok(mode != WorkerSendMode::Steer)
    }
    fn respond(&mut self, _: WorkerInputResponse) -> Result<(), String> {
        Err("Cursor SDK has no interactive approval flow".into())
    }
    fn abort(&mut self) -> Result<(), String> {
        self.cancel_queue();
        if self.stream.is_some() {
            self.cancelled = Some(Instant::now());
            self.cancel_run()?;
        }
        Ok(())
    }
    fn select_model(&mut self, provider: &str, model: &str) -> Result<(), String> {
        if provider != Backend::Cursor.as_str() {
            return Err("Cursor SDK requires a Cursor model".into());
        }
        self.model = self
            .models
            .iter()
            .find(|m| m.id == model)
            .cloned()
            .ok_or_else(|| format!("Unknown Cursor SDK model: {model}"))?;
        self.selection_changed();
        Ok(())
    }
    fn select_effort(&mut self, effort: &str) -> Result<(), String> {
        if !self.model.efforts.iter().any(|e| e == effort) {
            return Err(format!("Unsupported Cursor SDK effort: {effort}"));
        }
        let key = self
            .model
            .effort_parameter
            .clone()
            .ok_or("Cursor model has no effort parameter")?;
        self.model.set_parameter(&key, effort);
        self.selection_changed();
        Ok(())
    }
    fn select_service_tier(&mut self, tier: &str) -> Result<(), String> {
        if !self.model.tiers.iter().any(|t| t == tier) {
            return Err(format!("Unsupported Cursor SDK service tier: {tier}"));
        }
        self.model
            .set_parameter("fast", if tier == "priority" { "true" } else { "false" });
        self.selection_changed();
        Ok(())
    }
    fn model_selection(&self) -> Option<WorkerModelSelection> {
        Some(WorkerModelSelection {
            model: Some((Backend::Cursor.as_str().into(), self.model.id.clone())),
            effort: self.model.effort(),
        })
    }
    fn select_mode(&mut self, mode: &str) -> Result<(), String> {
        if !matches!(mode, "agent" | "plan") {
            return Err(format!("Unknown Cursor SDK mode: {mode}"));
        }
        self.mode = mode.into();
        self.events
            .push_back(WorkerEvent::Activity(WorkerActivity::ModeChanged(
                mode.into(),
            )));
        Ok(())
    }
    fn poll(&mut self) -> Option<WorkerEvent> {
        // SDK status/keepalive frames need not produce a UI event. Drain them
        // until an event or an empty channel: wakeups can cover several frames.
        loop {
            self.poll_metadata();
            self.poll_steering();
            if self.steering.is_empty()
                && let Some(error) = self.pending_finish.take()
            {
                self.finish(error);
            }
            if let Some(event) = self.events.pop_front() {
                return Some(event);
            }
            if self.pending_finish.is_none()
                && self
                    .cancelled
                    .is_some_and(|at| at.elapsed() > Duration::from_secs(30))
            {
                self.finish(Some("Cursor SDK cancellation timed out".into()));
                continue;
            }
            if let Some(stream) = &self.stream {
                match stream.try_recv() {
                    Ok(Ok(Some(value))) => {
                        if let Err(error) = self.envelope(value) {
                            self.finish(Some(error));
                        }
                    }
                    Ok(Ok(None)) => self.finish(None),
                    Ok(Err(error)) => self.finish(Some(error)),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.finish(Some("Cursor SDK stream reader stopped".into()))
                    }
                    Err(mpsc::TryRecvError::Empty) => return None,
                }
            } else if self.pending_finish.is_some() {
                return None;
            } else if let Some(input) = self.queue.pop_front() {
                let id = input.submission_id.clone();
                if let Err(error) = self.start_turn(input) {
                    if let Some(submission_id) = id {
                        self.events
                            .push_back(WorkerEvent::PromptCancelled { submission_id });
                    }
                    self.cancel_queue();
                    self.events.push_back(WorkerEvent::Failed(error));
                }
            } else if let Some(message) = self.identity.as_ref().and_then(|i| i.try_recv()) {
                if let Err(error) = self.send_peer_message(&message, WorkerSendMode::Prompt) {
                    self.events.push_back(WorkerEvent::Failed(error));
                }
            } else {
                return None;
            }
        }
    }
    fn close(&mut self) -> Result<(), String> {
        let _ = self.abort();
        self.stream = None;
        self.metadata_request = None;
        self.metadata_dirty = false;
        self.cancel_queue();
        self.bridge.close()
    }
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
