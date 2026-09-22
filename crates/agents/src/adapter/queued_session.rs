//! Backend-neutral ownership of main-session input until a delivery boundary.
use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::{Value, json};

use super::handler::IdempotencyBookkeeping;
use super::prompt_boundary::{Boundary, PromptBoundary};
use crate::{
    SessionCommand, SessionEvent, SessionOperation, SessionResponse,
    SessionResponsePayload as Payload, SessionTransport,
    extensions::{ExtensionUiResponse, PromptImage, PromptMode},
};

#[derive(Clone, Copy)]
pub(super) enum SteeringBoundary {
    /// The backend has no steering support; Enter becomes a follow-up.
    Unsupported,
    /// Native steering; follow-ups still belong to the shared queue.
    Native,
    /// A blocking hook stays open until native admission acknowledges the steer.
    Held,
    /// The native hook ends the loop after the completed tool batch, without abort.
    StopAfterBatch,
}

struct Input {
    id: String,
    mode: PromptMode,
    requested_mode: PromptMode,
    message: String,
    images: Vec<PromptImage>,
}

impl Input {
    fn receipt(&self, status: &str) -> SessionEvent {
        let mut content = vec![json!({"type":"text", "text":self.message})];
        content.extend(self.images.iter().map(|image| {
            json!({
                "type":"image", "data":image.data, "mimeType":image.mime_type,
            })
        }));
        activity(json!({"type":"prompt_delivery", "submissionId":self.id,
            "status":status, "message":{"role":"user", "content":content,
                "queued":true, "deliveryTracked":true,
                "promptMode": if self.mode == PromptMode::Steer {"steer"} else {"follow_up"}}}))
    }
}

struct Dispatch {
    inputs: Vec<Input>,
    tracks_delivery: bool,
    responded: bool,
    delivered: bool,
    starts_run: bool,
}

struct HeldBatch {
    boundary: Boundary,
    pending: HashSet<String>,
}

pub(super) struct QueuedSession {
    inner: Box<dyn SessionTransport>,
    policy: SteeringBoundary,
    hook: Option<PromptBoundary>,
    session_id: Option<String>,
    running: bool,
    compacting: bool,
    normal_requests: HashSet<String>,
    queue: VecDeque<Input>,
    /// Snapshot claimed at a stop-after-batch hook; no longer individually cancellable.
    stopped_batch: Vec<Input>,
    held_batch: Option<HeldBatch>,
    dispatched: HashMap<String, Dispatch>,
    pending: VecDeque<SessionEvent>,
    native_queue: Value,
    bookkeeping: IdempotencyBookkeeping,
}

impl QueuedSession {
    pub fn new(
        inner: Box<dyn SessionTransport>,
        policy: SteeringBoundary,
        hook: Option<PromptBoundary>,
    ) -> Self {
        Self {
            inner,
            policy,
            hook,
            session_id: None,
            running: false,
            compacting: false,
            normal_requests: HashSet::new(),
            queue: VecDeque::new(),
            stopped_batch: Vec::new(),
            held_batch: None,
            dispatched: HashMap::new(),
            pending: VecDeque::new(),
            native_queue: json!({}),
            bookkeeping: IdempotencyBookkeeping::default(),
        }
    }

    fn queue_changed(&mut self) {
        if let Some(hook) = &self.hook {
            hook.enable(
                self.queue
                    .iter()
                    .any(|input| input.mode == PromptMode::Steer),
            );
        }
        let mut event = self.native_queue.clone();
        event["type"] = "queue_update".into();
        // Only this owner can promise removal before dispatch. Native queue
        // IDs and pending receipts are not evidence of cancellability.
        event["cancellableIds"] = self
            .queue
            .iter()
            .map(|input| input.id.clone())
            .collect::<Vec<_>>()
            .into();
        for (key, ids, mode) in [
            ("steering", "steeringIds", PromptMode::Steer),
            ("followUp", "followUpIds", PromptMode::FollowUp),
        ] {
            let mut texts = event[key].as_array().cloned().unwrap_or_default();
            let mut identities = event[ids].as_array().cloned().unwrap_or_default();
            identities.resize(texts.len(), Value::String(String::new()));
            for input in self.queue.iter().filter(|input| input.mode == mode) {
                texts.push(input.message.clone().into());
                identities.push(input.id.clone().into());
            }
            event[key] = texts.into();
            event[ids] = identities.into();
        }
        self.pending.push_back(activity(event));
    }

    fn dispatch(&mut self, input: Input) {
        // Automatic follow-ups start a normal run only when idle. Escape uses
        // dispatch_as to stage inputs for the explicit native handoff instead.
        let mode = if self.running {
            PromptMode::Steer
        } else {
            PromptMode::Normal
        };
        self.dispatch_as(input, mode);
    }

    fn dispatch_as(&mut self, input: Input, mode: PromptMode) {
        self.dispatch_batch(vec![input], mode);
    }

    fn dispatch_batch(&mut self, inputs: Vec<Input>, mode: PromptMode) {
        if inputs.is_empty() {
            return;
        }
        let starts_run = !self.running;
        let tracks_delivery = self.inner.tracks_prompt_delivery(mode);
        // Starting separate native requests can start the model before the
        // remaining steers arrive. One envelope makes the whole snapshot
        // available on the first model step, regardless of the backend.
        let message = inputs
            .iter()
            .map(|input| input.message.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let images = inputs
            .iter()
            .flat_map(|input| input.images.iter().cloned())
            .collect();
        match self.inner.send(SessionCommand::Prompt {
            mode,
            message,
            images,
        }) {
            Ok(id) => {
                self.running = true;
                self.dispatched.insert(
                    id,
                    Dispatch {
                        inputs,
                        tracks_delivery,
                        responded: false,
                        delivered: false,
                        starts_run,
                    },
                );
            }
            Err(error) => {
                for input in inputs {
                    self.admission_resolved(&input.id);
                    self.pending.push_back(input.receipt("rejected"));
                    self.pending
                        .push_back(SessionEvent::Response(SessionResponse::failure(
                            Some(input.id),
                            SessionOperation::Prompt(input.requested_mode),
                            error.clone(),
                        )));
                }
            }
        }
    }

    fn take_steers(&mut self) -> Vec<Input> {
        let (steers, followups) = self
            .queue
            .drain(..)
            .partition(|input| input.mode == PromptMode::Steer);
        self.queue = followups;
        steers.into()
    }

    fn boundary(&mut self, boundary: Boundary) {
        if self.session_id.as_deref() != Some(boundary.session_id.as_str()) {
            boundary.release(false);
            return;
        }
        // Another parallel tool hook may arrive while a steer is being admitted.
        // It need not consume another queue item to let the completed batch finish.
        if self.held_batch.is_some() || !self.stopped_batch.is_empty() {
            boundary.release(false);
            return;
        }
        if !matches!(
            self.policy,
            SteeringBoundary::Held | SteeringBoundary::StopAfterBatch
        ) {
            boundary.release(false);
            return;
        }
        let batch = self.take_steers();
        if batch.is_empty() {
            boundary.release(false);
            return;
        }
        match self.policy {
            SteeringBoundary::StopAfterBatch => {
                self.stopped_batch = batch;
                boundary.release(true);
            }
            SteeringBoundary::Held => {
                // Snapshot all currently pending steers. Keep their identities
                // and order; follow-ups still wait for the run to settle.
                self.held_batch = Some(HeldBatch {
                    boundary,
                    pending: batch.iter().map(|input| input.id.clone()).collect(),
                });
                for input in batch {
                    self.dispatch(input);
                }
            }
            SteeringBoundary::Native | SteeringBoundary::Unsupported => {
                unreachable!("checked hook policy");
            }
        }
        self.queue_changed();
    }

    fn admission_resolved(&mut self, id: &str) {
        if let Some(batch) = &mut self.held_batch {
            batch.pending.remove(id);
            if batch.pending.is_empty() {
                self.release_boundary();
            }
        }
    }

    fn release_boundary(&mut self) {
        if let Some(batch) = self.held_batch.take() {
            batch.boundary.release(false);
        }
    }

    fn observe(&mut self, mut event: SessionEvent) -> Option<SessionEvent> {
        match &mut event {
            SessionEvent::Response(response) => {
                if let Ok(Payload::LoadState(state)) = &mut response.result {
                    self.session_id = Some(state.session_id.clone());
                    // A state reply can describe the instant before a prompt
                    // was admitted. Only settlement/rejection ends our run.
                    self.running |= state.is_streaming;
                    self.compacting = state.is_compacting;
                    state.pending_message_count += self.queue.len();
                }
                if response
                    .id
                    .as_ref()
                    .is_some_and(|id| self.normal_requests.remove(id))
                    && response.result.as_ref().is_err_and(|error| {
                        error.kind == crate::SessionResponseErrorKind::RejectedBeforeAcceptance
                    })
                {
                    self.running = false;
                }
                if let Some(id) = response.id.clone()
                    && let Some(mut dispatch) = self.dispatched.remove(&id)
                {
                    dispatch.responded = true;
                    for input in &dispatch.inputs {
                        self.admission_resolved(&input.id);
                        let mut reply = response.clone();
                        reply.id = Some(input.id.clone());
                        match &mut reply.result {
                            Ok(payload) => {
                                *payload = Payload::Prompt(input.requested_mode);
                                if !dispatch.tracks_delivery {
                                    let SessionEvent::Activity(receipt) = input.receipt("accepted")
                                    else {
                                        unreachable!()
                                    };
                                    let mut receipt = receipt.value().clone();
                                    receipt["message"]["deliveryTracked"] = false.into();
                                    self.pending.push_back(activity(receipt));
                                }
                            }
                            Err(error) => {
                                error.operation = SessionOperation::Prompt(input.requested_mode);
                                if dispatch.starts_run
                                    && error.kind
                                        == crate::SessionResponseErrorKind::RejectedBeforeAcceptance
                                {
                                    self.running = false;
                                }
                            }
                        }
                        self.pending.push_back(SessionEvent::Response(reply));
                    }
                    // Keep the identity mapping until delivery; an admission reply
                    // is not proof that the backend consumed the message.
                    let unknown = response.result.as_ref().is_err_and(|error| {
                        error.kind == crate::SessionResponseErrorKind::DeliveryUnknown
                    });
                    if (response.result.is_ok() || unknown)
                        && dispatch.tracks_delivery
                        && !dispatch.delivered
                    {
                        self.dispatched.insert(id, dispatch);
                    }
                    return None;
                }
            }
            SessionEvent::Activity(value) => {
                let mut body = value.value().clone();
                match body["type"].as_str() {
                    Some("agent_start") => self.running = true,
                    Some("agent_settled") => self.running = false,
                    Some("compaction_start") => self.compacting = true,
                    Some("compaction_end") => self.compacting = false,
                    Some("queue_update") => {
                        for (texts, ids) in
                            [("steering", "steeringIds"), ("followUp", "followUpIds")]
                        {
                            let claimed = body[ids]
                                .as_array()
                                .map(|ids| {
                                    ids.iter()
                                        .enumerate()
                                        .filter_map(|(index, id)| {
                                            self.dispatched
                                                .contains_key(id.as_str()?)
                                                .then_some(index)
                                        })
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default();
                            for index in claimed.into_iter().rev() {
                                if let Some(values) = body[ids].as_array_mut() {
                                    values.remove(index);
                                }
                                if let Some(values) = body[texts].as_array_mut()
                                    && index < values.len()
                                {
                                    values.remove(index);
                                }
                            }
                        }
                        self.native_queue = body;
                        self.queue_changed();
                        return None;
                    }
                    Some("prompt_delivery") => {
                        if body["submissionId"]
                            .as_str()
                            .is_some_and(|id| self.bookkeeping.contains(id))
                        {
                            return None;
                        }
                        if let Some(id) = body["submissionId"].as_str().map(str::to_owned)
                            && let Some(dispatch) = self.dispatched.get_mut(&id)
                        {
                            let status = body["status"].as_str().unwrap_or("unknown").to_owned();
                            dispatch.delivered |= status == "delivered";
                            if status == "delivered" {
                                self.bookkeeping.record_reached_model(id.clone());
                            }
                            let resolved = matches!(
                                status.as_str(),
                                "accepted" | "delivered" | "rejected" | "unknown"
                            );
                            let input_ids = dispatch
                                .inputs
                                .iter()
                                .map(|input| input.id.clone())
                                .collect::<Vec<_>>();
                            // The native receipt covers exactly the envelope
                            // we sent. Project each member's original payload,
                            // never the joined text or another member's images.
                            for input in &dispatch.inputs {
                                let SessionEvent::Activity(receipt) = input.receipt(&status) else {
                                    unreachable!()
                                };
                                let mut member = body.clone();
                                member["submissionId"] = input.id.clone().into();
                                member["message"] = receipt.value()["message"].clone();
                                self.pending.push_back(activity(member));
                            }
                            // The terminal response can follow the receipt; retain
                            // the mapping for it as well.
                            if dispatch.responded && dispatch.delivered {
                                self.dispatched.remove(&id);
                            }
                            if resolved {
                                for input_id in input_ids {
                                    self.admission_resolved(&input_id);
                                }
                            }
                            return None;
                        }
                    }
                    _ => {}
                }
            }
            SessionEvent::Failure(_) => {
                self.release_boundary();
                // A transport failure is not a user cancellation. Keep unsent
                // input available to the runtime's durable recovery path.
            }
            _ => {}
        }
        Some(event)
    }

    fn cancel_local(&mut self) {
        for input in self.queue.drain(..) {
            self.pending.push_back(input.receipt("cancelled"));
            self.pending
                .push_back(SessionEvent::Response(SessionResponse::cancelled(
                    input.id,
                    SessionOperation::Prompt(input.requested_mode),
                    "Cancelled before delivery".into(),
                )));
        }
        self.queue_changed();
    }
}

impl SessionTransport for QueuedSession {
    fn tracks_prompt_delivery(&self, mode: PromptMode) -> bool {
        self.inner.tracks_prompt_delivery(mode)
    }
    fn sandbox_adapter(&self) -> Option<&str> {
        self.inner.sandbox_adapter()
    }
    fn sandbox_mode(&self) -> Option<crate::HarnessAccessMode> {
        self.inner.sandbox_mode()
    }
    fn clear_queue(&mut self) -> Result<(), String> {
        self.cancel_local();
        Ok(())
    }
    fn cancel_prompt(&mut self, id: &str) -> Result<(), String> {
        if let Some(index) = self.queue.iter().position(|input| input.id == id) {
            let input = self.queue.remove(index).expect("located input");
            self.pending.push_back(input.receipt("cancelled"));
            self.pending
                .push_back(SessionEvent::Response(SessionResponse::cancelled(
                    input.id,
                    SessionOperation::Prompt(input.requested_mode),
                    "Cancelled before delivery".into(),
                )));
            self.queue_changed();
            return Ok(());
        }
        // A click can outlive the queue snapshot that offered it. Once this
        // owner has claimed the input (or never owned it), leave delivery alone.
        // In particular, do not turn a stale click into native cancellation or
        // fabricate a cancelled receipt. Only removal above proves cancellation.
        Ok(())
    }
    fn send(&mut self, command: SessionCommand) -> Result<String, String> {
        match command {
            SessionCommand::Prompt {
                mut mode,
                message,
                images,
            } if mode != PromptMode::Normal => {
                let requested_mode = mode;
                if mode == PromptMode::Steer && matches!(self.policy, SteeringBoundary::Native) {
                    let id = format!("farcaster-queued-{}", uuid::Uuid::new_v4());
                    let images = images
                        .into_iter()
                        .map(PromptImage::into_inline)
                        .collect::<Result<Vec<_>, _>>()?;
                    match self.inner.send(SessionCommand::Prompt {
                        mode,
                        message: message.clone(),
                        images: images.clone(),
                    }) {
                        Ok(native_id) => return Ok(native_id),
                        Err(error) if native_steer_turn_race(&error) => {
                            // Codex can finish its turn between our state poll
                            // and turn/steer. Keep the logical steer and run it
                            // on the next turn instead of exposing that backend
                            // timing race as a failed prompt.
                            self.running = false;
                            self.queue.push_back(Input {
                                id: id.clone(),
                                mode,
                                requested_mode,
                                message,
                                images,
                            });
                            self.queue_changed();
                            return Ok(id);
                        }
                        Err(error) => return Err(error),
                    }
                }
                if mode == PromptMode::Steer && matches!(self.policy, SteeringBoundary::Unsupported)
                {
                    mode = PromptMode::FollowUp;
                }
                let id = format!("farcaster-queued-{}", uuid::Uuid::new_v4());
                let images = images
                    .into_iter()
                    .map(PromptImage::into_inline)
                    .collect::<Result<_, _>>()?;
                self.queue.push_back(Input {
                    id: id.clone(),
                    mode,
                    requested_mode,
                    message,
                    images,
                });
                self.queue_changed();
                Ok(id)
            }
            SessionCommand::Abort => {
                self.cancel_local();
                for input in self.stopped_batch.drain(..) {
                    self.pending.push_back(input.receipt("cancelled"));
                    self.pending
                        .push_back(SessionEvent::Response(SessionResponse::cancelled(
                            input.id,
                            SessionOperation::Prompt(input.requested_mode),
                            "Cancelled before delivery".into(),
                        )));
                }
                self.inner.send(SessionCommand::Abort)
            }
            SessionCommand::ApplySteering => {
                // Escape is an explicit immediate handoff, unlike an ordinary
                // steer. Stage every input with its native queued mode before
                // invoking the adapter's interrupt-and-resume control.
                for input in std::mem::take(&mut self.stopped_batch) {
                    let mode = input.mode;
                    self.dispatch_as(input, mode);
                }
                while let Some(input) = self.queue.pop_front() {
                    let mode = input.mode;
                    self.dispatch_as(input, mode);
                }
                self.queue_changed();
                // A native handoff must not wait on our own post-tool hook.
                self.release_boundary();
                if let Some(hook) = &self.hook {
                    while let Some(boundary) = hook.poll() {
                        boundary.release(false);
                    }
                }
                // Keep the real control response (and any failure), including
                // when only an already-admitted native steer is pending.
                self.inner.send(SessionCommand::ApplySteering)
            }
            other => {
                let starts = matches!(other, SessionCommand::Prompt { .. });
                let id = self.inner.send(other)?;
                self.running |= starts;
                if starts {
                    self.normal_requests.insert(id.clone());
                }
                Ok(id)
            }
        }
    }
    fn respond(&mut self, response: ExtensionUiResponse) -> Result<(), String> {
        self.inner.respond(response)
    }
    fn poll(&mut self) -> Option<SessionEvent> {
        if let Some(event) = self.pending.pop_front() {
            return Some(event);
        }
        // Drain native state before deciding whether the session is idle.
        if let Some(event) = self.inner.poll() {
            return self.observe(event).or_else(|| self.pending.pop_front());
        }
        if let Some(boundary) = self.hook.as_ref().and_then(PromptBoundary::poll) {
            self.boundary(boundary);
        }
        if !self.running && !self.compacting {
            let batch = if self.stopped_batch.is_empty() {
                self.take_steers()
            } else {
                std::mem::take(&mut self.stopped_batch)
            };
            if !batch.is_empty() {
                self.queue_changed();
                self.dispatch_batch(batch, PromptMode::Normal);
            } else if let Some(input) = self.queue.pop_front() {
                self.queue_changed();
                self.dispatch(input);
            }
        }
        self.pending.pop_front()
    }
    fn close(&mut self) -> Result<(), String> {
        self.cancel_local();
        self.release_boundary();
        self.hook.take();
        self.inner.close()
    }
}

fn activity(value: Value) -> SessionEvent {
    SessionEvent::Activity(value.into())
}

fn native_steer_turn_race(error: &str) -> bool {
    error.contains("has not reported its active turn")
        || error.contains("no active turn to steer")
        || error.contains("expected active turn id")
}

#[cfg(test)]
#[path = "queued_session_tests.rs"]
mod tests;
