use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::{Value, json};

use super::handler::IdempotencyBookkeeping;
use super::prompt_boundary::{Boundary, PromptBoundary};
use super::prompt_input::{PromptInput, prompt_mode, worker_mode};
use super::prompt_queue::{PromptQueue, QueuedPrompt};
use crate::{
    DeliveryStatus, SessionCommand, SessionEvent, SessionOperation, SessionResponse,
    SessionResponsePayload as Payload, SessionTransport, SteerErrorRecovery,
    extensions::{ExtensionUiResponse, PromptImage, PromptMode},
};

const MAX_FILTERED_EVENTS_PER_POLL: usize = 256;

#[derive(Clone, Copy)]
pub(super) enum SteeringBoundary {
    Unsupported,
    Native,
    Held,
    StopAfterBatch,
}

struct Input {
    prompt: PromptInput,
    requested_mode: PromptMode,
}

impl Input {
    fn new(
        id: String,
        mode: PromptMode,
        requested_mode: PromptMode,
        message: String,
        images: Vec<PromptImage>,
    ) -> Self {
        Self {
            prompt: PromptInput {
                submission_id: Some(id),
                mode: worker_mode(mode),
                message,
                images,
            },
            requested_mode,
        }
    }

    fn id(&self) -> &str {
        self.prompt
            .submission_id
            .as_deref()
            .expect("queued prompt identity")
    }

    fn mode(&self) -> PromptMode {
        prompt_mode(self.prompt.mode)
    }

    fn receipt(&self, status: DeliveryStatus) -> crate::contract::PromptDelivery {
        self.prompt.receipt(status)
    }
}

impl QueuedPrompt for Input {
    fn input(&self) -> &PromptInput {
        &self.prompt
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
    queue: PromptQueue<Input>,
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
            queue: PromptQueue::default(),
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
                    .any(|input| input.mode() == PromptMode::Steer),
            );
        }
        let mut event = self.native_queue.clone();
        event["type"] = "queue_update".into();
        event["cancellableIds"] = self
            .queue
            .iter()
            .map(|input| input.id().to_owned())
            .collect::<Vec<_>>()
            .into();
        for (key, ids, mode) in [
            ("steering", "steeringIds", PromptMode::Steer),
            ("followUp", "followUpIds", PromptMode::FollowUp),
        ] {
            let mut texts = event[key].as_array().cloned().unwrap_or_default();
            let mut identities = event[ids].as_array().cloned().unwrap_or_default();
            identities.resize(texts.len(), Value::String(String::new()));
            for input in self.queue.iter().filter(|input| input.mode() == mode) {
                texts.push(input.prompt.message.clone().into());
                identities.push(input.id().to_owned().into());
            }
            event[key] = texts.into();
            event[ids] = identities.into();
        }
        self.pending.push_back(activity(event));
    }

    fn dispatch(&mut self, input: Input) {
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
        let message = inputs
            .iter()
            .map(|input| input.prompt.message.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let images = inputs
            .iter()
            .flat_map(|input| input.prompt.images.iter().cloned())
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
                    self.admission_resolved(input.id());
                    self.pending.push_back(SessionEvent::Activity(
                        input.receipt(DeliveryStatus::Rejected).into(),
                    ));
                    self.pending
                        .push_back(SessionEvent::Response(SessionResponse::failure(
                            Some(input.id().to_owned()),
                            SessionOperation::Prompt(input.requested_mode),
                            error.clone(),
                        )));
                }
            }
        }
    }

    fn boundary(&mut self, boundary: Boundary) {
        if self.session_id.as_deref() != Some(boundary.session_id.as_str()) {
            boundary.release(false);
            return;
        }
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
        let batch = self.queue.take_steers();
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
                self.held_batch = Some(HeldBatch {
                    boundary,
                    pending: batch.iter().map(|input| input.id().to_owned()).collect(),
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
                        self.admission_resolved(input.id());
                        let mut reply = response.clone();
                        reply.id = Some(input.id().to_owned());
                        match &mut reply.result {
                            Ok(payload) => {
                                *payload = Payload::Prompt(input.requested_mode);
                                if !dispatch.tracks_delivery {
                                    let mut receipt = input.receipt(DeliveryStatus::Accepted);
                                    receipt
                                        .message
                                        .as_mut()
                                        .expect("input message")
                                        .delivery_tracked = false;
                                    self.pending
                                        .push_back(SessionEvent::Activity(receipt.into()));
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
                if value.kind() == &crate::SessionActivityKind::PromptDelivery {
                    let receipt = value.prompt_delivery()?;
                    let id = &receipt.submission_id;
                    if self.bookkeeping.contains(id) {
                        return None;
                    }
                    if let Some(dispatch) = self.dispatched.get_mut(id) {
                        let status = receipt.status;
                        dispatch.delivered |= status == DeliveryStatus::Delivered;
                        if status == DeliveryStatus::Delivered {
                            self.bookkeeping.record_reached_model(id.clone());
                        }
                        let resolved = matches!(
                            status,
                            DeliveryStatus::Accepted
                                | DeliveryStatus::Delivered
                                | DeliveryStatus::Rejected
                                | DeliveryStatus::Unknown
                        );
                        let input_ids = dispatch
                            .inputs
                            .iter()
                            .map(|input| input.id().to_owned())
                            .collect::<Vec<_>>();
                        for input in &dispatch.inputs {
                            self.pending
                                .push_back(SessionEvent::Activity(input.receipt(status).into()));
                        }
                        if dispatch.responded && dispatch.delivered {
                            self.dispatched.remove(id);
                        }
                        if resolved {
                            for input_id in input_ids {
                                self.admission_resolved(&input_id);
                            }
                        }
                        return None;
                    }
                    return Some(event);
                }
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
                    _ => {}
                }
            }
            SessionEvent::Failure(_) => {
                self.release_boundary();
            }
            _ => {}
        }
        Some(event)
    }

    fn cancel_local(&mut self) {
        let inputs = self.queue.take_all();
        self.cancel_inputs(inputs);
        self.queue_changed();
    }

    fn cancel_inputs(&mut self, inputs: impl IntoIterator<Item = Input>) {
        for input in inputs {
            self.pending.push_back(SessionEvent::Activity(
                input.receipt(DeliveryStatus::Cancelled).into(),
            ));
            self.pending
                .push_back(SessionEvent::Response(SessionResponse::cancelled(
                    input.id().to_owned(),
                    SessionOperation::Prompt(input.requested_mode),
                    "Cancelled before delivery".into(),
                )));
        }
    }
}

impl SessionTransport for QueuedSession {
    fn has_exited(&mut self) -> bool {
        self.inner.has_exited()
    }

    fn retain_inbox(&self) -> Result<Option<Box<dyn crate::SessionInbox>>, String> {
        self.inner.retain_inbox()
    }

    fn can_retire(&self) -> bool {
        !self.running
            && !self.compacting
            && self.normal_requests.is_empty()
            && self.queue.is_empty()
            && self.stopped_batch.is_empty()
            && self.held_batch.is_none()
            && self.dispatched.is_empty()
            && self.pending.is_empty()
            && self.inner.can_retire()
    }

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
        if let Some(input) = self.queue.cancel(id) {
            self.cancel_inputs([input]);
            self.queue_changed();
            return Ok(());
        }
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
                        Err(error) => {
                            let recovery = self.inner.steer_error_recovery(&error);
                            if recovery == SteerErrorRecovery::Fail {
                                return Err(error);
                            }
                            self.running = recovery == SteerErrorRecovery::RetryWhenIdle;
                            self.queue.push(Input::new(
                                id.clone(),
                                mode,
                                requested_mode,
                                message,
                                images,
                            ));
                            self.queue_changed();
                            return Ok(id);
                        }
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
                self.queue.push(Input::new(
                    id.clone(),
                    mode,
                    requested_mode,
                    message,
                    images,
                ));
                self.queue_changed();
                Ok(id)
            }
            SessionCommand::Abort => {
                self.cancel_local();
                let inputs = std::mem::take(&mut self.stopped_batch);
                self.cancel_inputs(inputs);
                self.inner.send(SessionCommand::Abort)
            }
            SessionCommand::ApplySteering => {
                for input in std::mem::take(&mut self.stopped_batch) {
                    let mode = input.mode();
                    self.dispatch_as(input, mode);
                }
                while let Some(input) = self.queue.pop() {
                    let mode = input.mode();
                    self.dispatch_as(input, mode);
                }
                self.queue_changed();
                self.release_boundary();
                if let Some(hook) = &self.hook {
                    while let Some(boundary) = hook.poll() {
                        boundary.release(false);
                    }
                }
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
        let mut filtered = 0;
        while let Some(event) = self.inner.poll() {
            if let Some(event) = self.observe(event).or_else(|| self.pending.pop_front()) {
                return Some(event);
            }
            filtered += 1;
            if filtered == MAX_FILTERED_EVENTS_PER_POLL {
                std::thread::current().unpark();
                return None;
            }
        }
        if let Some(boundary) = self.hook.as_ref().and_then(PromptBoundary::poll) {
            self.boundary(boundary);
        }
        if !self.running && !self.compacting {
            let batch = if self.stopped_batch.is_empty() {
                self.queue.take_steers()
            } else {
                std::mem::take(&mut self.stopped_batch)
            };
            if !batch.is_empty() {
                self.queue_changed();
                self.dispatch_batch(batch, PromptMode::Normal);
            } else if let Some(input) = self.queue.pop() {
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

#[cfg(test)]
#[path = "queued_session_tests.rs"]
mod tests;
