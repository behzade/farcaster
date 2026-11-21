use super::*;
use farcaster_storage::{SessionRepository, TranscriptSession};
use std::path::Path;

const CHECKPOINT_INTERVAL: Duration = Duration::from_millis(500);

/// The process owns this address. A different document may be selected for display.
pub(super) struct LiveTranscript {
    harness: Backend,
    path: Option<PathBuf>,
    project: PathBuf,
    profile_id: Option<String>,
    complete_from_start: bool,
    startup_revision: Option<i64>,
    startup_history: Option<Arc<Vec<Value>>>,
    messages: PendingItems,
    open_message: Option<String>,
    content_indices: Vec<usize>,
    tools: HashMap<String, Value>,
    inputs: HashMap<String, Value>,
    next_write: Option<Instant>,
    settled: bool,
}

#[derive(Default)]
struct PendingItems {
    order: Vec<String>,
    items: HashMap<String, Value>,
    // Only open stream items remain after a successful checkpoint.
    dirty: HashSet<String>,
}

impl PendingItems {
    fn put(&mut self, key: String, value: Value) {
        if self.dirty.insert(key.clone()) {
            self.order.push(key.clone());
        }
        self.items.insert(key, value);
    }

    fn edit(&mut self, key: &str) -> Option<&mut Value> {
        let value = self.items.get_mut(key)?;
        if self.dirty.insert(key.to_owned()) {
            self.order.push(key.to_owned());
        }
        Some(value)
    }
}

impl LiveTranscript {
    pub(super) fn new(
        harness: Backend,
        path: Option<&Path>,
        project: &Path,
        profile_id: Option<&str>,
        complete_from_start: bool,
        store: &SharedStateStore,
    ) -> Self {
        let mut capture = Self {
            harness,
            path: path.map(Path::to_owned),
            project: project.to_owned(),
            profile_id: profile_id.map(str::to_owned),
            complete_from_start,
            startup_revision: None,
            startup_history: None,
            messages: Default::default(),
            open_message: None,
            content_indices: Vec::new(),
            tools: HashMap::new(),
            inputs: HashMap::new(),
            next_write: None,
            settled: false,
        };
        if let Some(session) = capture.address() {
            match store.load_transcript(session, Some(0)) {
                Ok(prior) => capture.startup_revision = Some(prior.map_or(0, |p| p.revision)),
                Err(error) => {
                    zlog::warn!("Read transcript revision before resume: {error}");
                }
            }
        } else {
            // A new/forked backend locator has no prior document.
            capture.startup_revision = Some(0);
        }
        capture
    }

    fn address(&self) -> Option<TranscriptSession<'_>> {
        Some(TranscriptSession {
            harness: self.harness,
            path: self.path.as_deref()?,
            project: &self.project,
            profile_id: self.profile_id.as_deref(),
        })
    }

    pub(super) fn bind(&mut self, path: &Path, store: &SharedStateStore) {
        if self.path.as_deref().is_some_and(|old| old != path) {
            // A native switch can move the transport to another session. Flush the
            // original row by its original address before binding subsequent events.
            if let Err(error) = self.flush(store) {
                zlog::warn!("Save transcript before native session change: {error}");
            }
            *self = Self::new(
                self.harness,
                Some(path),
                &self.project,
                self.profile_id.as_deref(),
                false,
                store,
            );
        } else {
            self.path = Some(path.to_owned());
        }
    }

    pub(super) fn replace_startup_history(&mut self, messages: &Arc<Vec<Value>>) {
        if self.startup_revision.is_some() && self.messages.dirty.is_empty() {
            self.startup_history = Some(Arc::clone(messages));
            self.next_write = Some(Instant::now());
        }
    }

    pub(super) fn observe(&mut self, event: &agents::contract::SessionActivity) {
        let value = event.value();
        use SessionActivityKind as Kind;
        match event.kind() {
            Kind::PromptDelivery => self.delivery(event),
            Kind::PeerMessage => {
                if let (Some(from), Some(message)) =
                    (value["from"].as_str(), value["message"].as_str())
                {
                    let peer = farcaster_agent_protocol::PeerMessage {
                        from: from.to_owned(),
                        message: message.to_owned(),
                    };
                    self.messages
                        .put(local_key(), json!({"role":"user","content":peer.prompt()}));
                }
            }
            Kind::MessageStarted | Kind::MessageEnded => {
                let Some(message) = value.get("message").filter(|message| message.is_object())
                else {
                    return;
                };
                let key = if event.kind() == &Kind::MessageEnded {
                    self.open_message
                        .take()
                        .unwrap_or_else(|| message_key(message))
                } else {
                    let key = message_key(message);
                    self.open_message = Some(key.clone());
                    self.content_indices.clear();
                    if let Some(content) = message["content"].as_array() {
                        self.content_indices.extend(0..content.len());
                    }
                    key
                };
                if message["role"] == "toolResult" {
                    if let Some(id) = message["toolCallId"].as_str() {
                        self.tool_result(
                            id,
                            message,
                            message["isError"].as_bool().unwrap_or(false),
                            event.kind() == &Kind::MessageEnded,
                        );
                    }
                } else {
                    self.messages.put(key, message.clone());
                }
            }
            Kind::MessageUpdated => {
                let Some(key) = &self.open_message else {
                    return;
                };
                if let Some(message) = self.messages.edit(key) {
                    update_content(message, &mut self.content_indices, value);
                }
            }
            Kind::ToolStarted
            | Kind::ToolMetadataChanged
            | Kind::ToolUpdated
            | Kind::ToolFinished => {
                let Some(id) = value["toolCallId"].as_str().filter(|id| !id.is_empty()) else {
                    return;
                };
                let call = self
                    .tools
                    .entry(id.to_owned())
                    .or_insert_with(|| json!({"type":"toolCall","id":id}));
                for (from, to) in [
                    ("toolName", "name"),
                    ("args", "arguments"),
                    ("toolMetadata", "toolMetadata"),
                ] {
                    if let Some(field) = value.get(from).filter(|field| !field.is_null()) {
                        call[to] = field.clone();
                    }
                }
                if event.kind() == &Kind::ToolMetadataChanged
                    && let Some(item) = self.messages.edit(&format!("tool-result:{id}"))
                {
                    item["farcasterToolCall"] = call.clone();
                    self.next_write
                        .get_or_insert_with(|| Instant::now() + CHECKPOINT_INTERVAL);
                    return;
                }
                let finished = event.kind() == &Kind::ToolFinished;
                let result = value
                    .get("result")
                    .or_else(|| value.get("partialResult"))
                    .unwrap_or(&Value::Null);
                self.tool_result(
                    id,
                    result,
                    value["isError"].as_bool().unwrap_or(false),
                    finished,
                );
            }
            Kind::AgentStarted => {
                self.settled = false;
                return;
            }
            Kind::AgentSettled => {
                self.settled = true;
                self.next_write = Some(Instant::now());
                return;
            }
            _ => return,
        }
        if !self.messages.dirty.is_empty() {
            self.next_write
                .get_or_insert_with(|| Instant::now() + CHECKPOINT_INTERVAL);
        }
    }

    fn delivery(&mut self, event: &agents::contract::SessionActivity) {
        let Some(receipt) = event.prompt_delivery() else {
            return;
        };
        let id = &receipt.submission_id;
        use agents::DeliveryStatus as Status;
        let pending = self.inputs.remove(id);
        // Rejected, cancelled and uncertain sends are handled by the durable outbox.
        // A late rejection must not overwrite an already accepted/delivered item.
        if !matches!(receipt.status, Status::Accepted | Status::Delivered) {
            return;
        }
        let Some(mut message) = receipt
            .message
            .as_ref()
            .map(|value| value.value())
            .or(pending)
        else {
            return;
        };
        if receipt.status == Status::Accepted {
            self.inputs.insert(id.clone(), message.clone());
        }
        if message["queued"].as_bool() == Some(true) && receipt.status != Status::Delivered {
            return;
        }
        message["role"] = "user".into();
        message["submissionId"] = id.clone().into();
        message["deliveryStatus"] =
            serde_json::to_value(receipt.status).expect("serializable receipt status");
        self.messages.put(format!("submit:{id}"), message);
    }

    fn tool_result(&mut self, id: &str, result: &Value, error: bool, finished: bool) {
        let key = format!("tool-result:{id}");
        let mut message = if result.is_null() {
            self.messages
                .items
                .remove(&key)
                .unwrap_or_else(|| json!({}))
        } else if result.is_object() {
            result.clone()
        } else {
            json!({"content": result})
        };
        message["role"] = "toolResult".into();
        message["toolCallId"] = id.into();
        message["farcasterToolState"] = if finished { "finished" } else { "running" }.into();
        message["isError"] = error.into();
        if let Some(call) = self.tools.get(id) {
            message["farcasterToolCall"] = call.clone();
            if let Some(name) = call.get("name") {
                message["toolName"] = name.clone();
            }
        }
        self.messages.put(key, message);
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.path.as_ref().and(self.next_write)
    }

    pub(super) fn flush(&mut self, repository: &dyn SessionRepository) -> Result<(), String> {
        let Some(address) = self.address() else {
            return Ok(());
        };
        if let (Some(history), Some(revision)) = (&self.startup_history, self.startup_revision) {
            repository.replace_transcript(address, revision, history, &Value::Null)?;
            self.startup_history = None;
            self.startup_revision = None;
        }
        if !self.messages.dirty.is_empty() {
            let items: Vec<_> = self
                .messages
                .order
                .iter()
                .map(|key| (key.clone(), self.messages.items[key].clone()))
                .collect();
            repository.upsert_transcript_items(
                self.address().expect("bound process"),
                &items,
                self.complete_from_start,
            )?;
            self.startup_revision = None;
            self.messages.order.clear();
            self.messages.dirty.clear();
        }
        self.messages.items.retain(|key, _| {
            self.open_message.as_ref() == Some(key)
                || (!self.settled && key.starts_with("tool-result:"))
        });
        if self.settled {
            self.tools.clear();
            self.settled = false;
        }
        self.next_write = None;
        Ok(())
    }
}

fn local_key() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn message_key(message: &Value) -> String {
    if let Some(id) = message["submissionId"].as_str() {
        return format!("submit:{id}");
    }
    local_key()
}

fn update_content(message: &mut Value, indices: &mut Vec<usize>, event: &Value) {
    let delta = &event["assistantMessageEvent"];
    let index = delta["contentIndex"]
        .as_u64()
        .and_then(|index| usize::try_from(index).ok());
    let kind = delta["type"].as_str().unwrap_or_default();
    if let (Some(index), Some((field, stage)), Some(blocks)) = (
        index,
        kind.split_once('_'),
        message["content"].as_array_mut(),
    ) && matches!(field, "text" | "thinking" | "toolcall")
    {
        // Content indices are provider-assigned and can be sparse. Keep compact
        // ordered blocks and append just the received text, even if the adapter
        // also included the complete message on every token.
        let position = indices.binary_search(&index).unwrap_or_else(|position| {
            indices.insert(position, index);
            blocks.insert(position, json!({}));
            position
        });
        let block = &mut blocks[position];
        if field == "toolcall" {
            if let Some(call) = delta
                .get("toolCall")
                .or_else(|| event["message"]["content"].get(index))
            {
                block.clone_from(call);
            }
        } else {
            if !block.is_object() {
                *block = json!({});
            }
            block["type"] = field.into();
            if stage == "delta" {
                if let Some(suffix) = delta["delta"].as_str() {
                    if !block[field].is_string() {
                        block[field] = "".into();
                    }
                    if let Value::String(text) = &mut block[field] {
                        text.push_str(suffix);
                    }
                }
            } else if let Some(content) = delta["content"].as_str() {
                block[field] = content.into();
            }
        }
    } else if let Some(full) = event.get("message")
        && let Some(blocks) = full["content"].as_array()
    {
        message.clone_from(full);
        indices.clear();
        indices.extend(0..blocks.len());
    }
}

impl RuntimeOwner {
    pub(super) fn flush_live_transcript(&mut self) {
        if let (Some(capture), Some(store)) = (&mut self.live_transcript, &self.state)
            && let Err(error) = capture.flush(store)
        {
            capture.next_write = Some(Instant::now() + CHECKPOINT_INTERVAL);
            zlog::warn!("Save live transcript: {error}");
        }
    }
}

#[cfg(test)]
#[path = "live_transcript_tests.rs"]
mod tests;
