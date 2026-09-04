use super::*;

impl ConversationState {
    #[allow(dead_code)] // Used by the standalone transcript benchmark.
    pub(crate) fn reduce(&mut self, event: &Value) -> Option<usize> {
        self.reduce_with_projection(event, true)
    }

    pub(crate) fn reduce_deferred(&mut self, event: &Value) -> Option<usize> {
        self.reduce_with_projection(event, false)
    }

    pub(crate) fn reduce_deferred_with_change(&mut self, event: &Value) -> (Option<usize>, bool) {
        let previous_state = (
            self.running,
            self.settled,
            self.compacting,
            self.retrying,
            self.queue.clone(),
            self.average_cache_hit_rate,
            self.diagnostics.len(),
        );
        let changed_from = self.reduce_deferred(event);
        let state_changed = previous_state
            != (
                self.running,
                self.settled,
                self.compacting,
                self.retrying,
                self.queue.clone(),
                self.average_cache_hit_rate,
                self.diagnostics.len(),
            );
        (changed_from, state_changed)
    }

    fn reduce_with_projection(&mut self, event: &Value, project_live: bool) -> Option<usize> {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let previous_len = self.items.len();
        let previous_live_start = self.live_message.map(|live| live.start);
        let mut incremental_content_changed = true;
        match kind {
            "agent_start" => {
                self.running = true;
                self.settled = false;
            }
            "agent_end" => {
                if event.get("willRetry").and_then(Value::as_bool) == Some(true) {
                    self.retrying = true;
                }
            }
            "agent_settled" => {
                self.running = false;
                self.settled = true;
                self.retrying = false;
                self.compacting = false;
            }
            "message_start" => self.start_message(event.get("message")),
            "message_update" => {
                incremental_content_changed =
                    self.update_message(event.get("assistantMessageEvent"), project_live);
            }
            "message_end" => self.end_message(event.get("message")),
            "peer_message" => self.peer_message(event),
            "tool_execution_start" => self.start_tool(event),
            "tool_execution_update" => incremental_content_changed = self.update_tool(event),
            "tool_execution_end" => self.end_tool(event),
            "tool_review_changed" => self.review_tool(event),
            "queue_update" => {
                self.queue.steering = strings(event.get("steering"));
                self.queue.follow_up = strings(event.get("followUp"));
            }
            "compaction_start" => {
                self.compacting = true;
                self.notice(format!(
                    "Compacting context ({})",
                    text_field(event, "reason")
                ));
            }
            "compaction_end" => {
                self.compacting = false;
                let message = if event.get("aborted").and_then(Value::as_bool) == Some(true) {
                    "Compaction aborted".to_owned()
                } else if let Some(error) = event.get("errorMessage").and_then(Value::as_str) {
                    format!("Compaction failed: {error}")
                } else {
                    "Context compacted".to_owned()
                };
                self.notice(message);
            }
            "auto_retry_start"
            | "summarization_retry_scheduled"
            | "summarization_retry_attempt_start" => {
                self.retrying = true;
                self.notice(retry_notice(event));
            }
            "auto_retry_end" | "summarization_retry_finished" => {
                self.retrying = false;
                self.notice(retry_notice(event));
            }
            "extension_error" => self.push_extension_error(text_field(event, "error")),
            "turn_start" | "turn_end" => {}
            unknown => self.diagnostic(format!("Unknown RPC event: {unknown}")),
        }
        match kind {
            "message_start" => Some(previous_len.saturating_sub(1)),
            "message_update" if incremental_content_changed => previous_live_start,
            "message_update" => None,
            "message_end" => {
                previous_live_start.map(|start| start.min(previous_len.saturating_sub(1)))
            }
            "peer_message" => Some(previous_len),
            "tool_execution_update" if !incremental_content_changed => None,
            "tool_execution_start"
            | "tool_execution_update"
            | "tool_execution_end"
            | "tool_review_changed" => event
                .get("toolCallId")
                .and_then(Value::as_str)
                .and_then(|id| self.tools.get(id).copied())
                .or(Some(previous_len)),
            "compaction_start"
            | "compaction_end"
            | "auto_retry_start"
            | "auto_retry_end"
            | "summarization_retry_scheduled"
            | "summarization_retry_attempt_start"
            | "summarization_retry_finished"
            | "extension_error" => Some(previous_len),
            _ => None,
        }
    }

    pub(super) fn start_message(&mut self, message: Option<&Value>) {
        self.content.clear();
        self.dirty_content.clear();
        self.projected_content.clear();
        if let Some(message) = message
            && message.get("role").and_then(Value::as_str) == Some("user")
        {
            self.queue.acknowledge(&message_text(message));
        }
        if message.is_some_and(|message| {
            message.get("role").and_then(Value::as_str) == Some("toolResult")
                && message
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| {
                        self.items.iter().any(|item| {
                            item.kind == TranscriptKind::Tool
                                && item.tool_call_id.as_deref() == Some(id)
                        })
                    })
        }) {
            self.live_message = Some(LiveMessage {
                start: self.items.len(),
                len: 0,
            });
            return;
        }
        let mut projected = message.map(project_message_items).unwrap_or_default();
        if projected.len() == 1
            && projected[0].kind == TranscriptKind::User
            && self.optimistic_user.is_some()
        {
            self.live_message = Some(LiveMessage {
                start: self.items.len(),
                len: 0,
            });
            return;
        }
        for item in &mut projected {
            item.streaming = true;
        }
        let start = self.items.len();
        let len = projected.len();
        self.items.extend(projected.into_iter().map(Arc::new));
        self.live_message = Some(LiveMessage { start, len });
    }

    fn update_message(&mut self, delta: Option<&Value>, project_live: bool) -> bool {
        let Some(delta) = delta else { return false };
        let Some(content_index) = delta
            .get("contentIndex")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
        else {
            return false;
        };
        let delta_type = delta
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let partial = self.content.entry(content_index).or_default();
        let changed = match delta_type {
            "text_start" => reset_partial(partial, PartialKind::Text),
            "text_delta" => append_delta(partial, delta),
            "text_end" => finish_content(partial, delta),
            "thinking_start" => reset_partial(partial, PartialKind::Thinking),
            "thinking_delta" => append_delta(partial, delta),
            "thinking_end" => finish_content(partial, delta),
            "toolcall_start" => {
                let label = delta
                    .get("toolCall")
                    .and_then(tool_name)
                    .or_else(|| delta.get("toolName").and_then(Value::as_str))
                    .map(display_tool_name)
                    .unwrap_or_default();
                let changed =
                    reset_partial(partial, PartialKind::ToolCall) || partial.label != label;
                partial.label = label;
                changed
            }
            "toolcall_delta" => append_delta(partial, delta),
            "toolcall_end" => delta.get("toolCall").is_some_and(|tool_call| {
                let label = tool_name(tool_call)
                    .map(display_tool_name)
                    .unwrap_or_else(|| "Tool".into());
                let value = tool_arguments(tool_call);
                let changed = partial.label != label || partial.value != value;
                partial.label = label;
                partial.value = value;
                changed
            }),
            _ => return false,
        };
        if !changed {
            return false;
        }
        self.dirty_content.insert(content_index);
        if project_live {
            self.flush_live_projection();
        }
        true
    }

    pub(crate) fn flush_live_projection(&mut self) {
        let dirty = std::mem::take(&mut self.dirty_content);
        for content_index in dirty {
            let projection_existed = self.projected_content.contains(&content_index);
            if self.projected_content.is_empty() {
                self.clear_initial_live_projection();
            }
            self.refresh_live_projection(content_index, projection_existed);
            self.projected_content.insert(content_index);
        }
    }

    fn clear_initial_live_projection(&mut self) {
        let Some(mut live) = self.live_message else {
            return;
        };
        self.items.splice(live.start..live.start + live.len, []);
        live.len = 0;
        self.live_message = Some(live);
    }

    fn refresh_live_projection(&mut self, content_index: usize, content_existed: bool) {
        let Some(mut live) = self.live_message else {
            return;
        };
        let Some(partial) = self.content.get(&content_index) else {
            return;
        };
        let position = self.content.range(..content_index).count();
        let projected = Arc::new(TranscriptItem {
            kind: match partial.kind {
                PartialKind::Text => TranscriptKind::Assistant,
                PartialKind::Thinking => TranscriptKind::Thinking,
                PartialKind::ToolCall => TranscriptKind::Tool,
            },
            label: match partial.kind {
                PartialKind::Text | PartialKind::Thinking => "",
                PartialKind::ToolCall if partial.label.is_empty() => "Tool",
                PartialKind::ToolCall => &partial.label,
            }
            .into(),
            text: partial.value.clone(),
            images: Arc::default(),
            stream_chunks: partial.chunks.clone(),
            streaming: true,
            is_error: false,
            tool_call_id: None,
            tool_output: String::new(),
            tool_presentation: None,
            tool_review: None,
            invocation: None,
        });
        if content_existed {
            self.items.set(live.start + position, projected);
        } else {
            self.items.insert(live.start + position, projected);
            live.len += 1;
            self.live_message = Some(live);
        }
    }

    pub(super) fn end_message(&mut self, message: Option<&Value>) {
        let Some(message) = message else { return };
        if message.get("role").and_then(Value::as_str) == Some("assistant") {
            self.record_cache_hit_rate(message);
        }
        if message.get("role").and_then(Value::as_str) == Some("toolResult") {
            if let Some(live) = self.live_message.take() {
                self.items.splice(live.start..live.start + live.len, []);
            }
            let id = message
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if let Some(index) = self.items.rposition(|item| {
                item.kind == TranscriptKind::Tool && item.tool_call_id.as_deref() == Some(id)
            }) {
                let mut item = self.items[index].clone();
                apply_tool_result(Arc::make_mut(&mut item), message, true);
                self.items.set(index, item);
                self.content.clear();
                self.dirty_content.clear();
                self.projected_content.clear();
                return;
            }
        }
        let mut final_items = project_message_items(message)
            .into_iter()
            .map(Arc::new)
            .collect::<Vec<_>>();
        let finalizes_user = final_items
            .iter()
            .any(|item| item.kind == TranscriptKind::User);
        if finalizes_user
            && let Some(optimistic) = self.optimistic_user.as_ref()
            && let Some(resolution) = optimistic
                .invocation
                .as_deref()
                .filter(|value| !value.is_empty())
            && let Some(final_user) = final_items
                .iter_mut()
                .find(|item| item.kind == TranscriptKind::User)
        {
            let final_user = Arc::make_mut(final_user);
            final_user.text.clone_from(&optimistic.text);
            final_user.invocation = Some(resolution.to_owned());
        }
        if let Some(live) = self.live_message.take() {
            self.items
                .splice(live.start..live.start + live.len, final_items);
        } else {
            self.items.extend(final_items);
        }
        if finalizes_user
            && let Some(optimistic) = self.optimistic_user.take()
            && let Some(index) = self.items.position(|item| Arc::ptr_eq(item, &optimistic))
        {
            self.items.remove(index);
        }
        self.content.clear();
        self.dirty_content.clear();
        self.projected_content.clear();
    }

    pub(super) fn record_cache_hit_rate(&mut self, message: &Value) {
        let Some((cache_read, prompt_tokens)) = cache_usage(message) else {
            return;
        };
        self.cache_read_tokens = self.cache_read_tokens.saturating_add(cache_read);
        self.prompt_tokens = self.prompt_tokens.saturating_add(prompt_tokens);
        self.average_cache_hit_rate =
            Some(self.cache_read_tokens as f64 / self.prompt_tokens as f64 * 100.0);
    }

    fn peer_message(&mut self, event: &Value) {
        self.items.push(Arc::new(peer_transcript_item(PeerMessage {
            from: text_field(event, "from"),
            message: text_field(event, "message"),
        })));
    }

    fn notice(&mut self, text: String) {
        self.items.push(Arc::new(TranscriptItem {
            kind: TranscriptKind::Notice,
            label: "Run".into(),
            text,
            images: Arc::default(),
            stream_chunks: Arc::default(),
            streaming: false,
            is_error: false,
            tool_call_id: None,
            tool_output: String::new(),
            tool_presentation: None,
            tool_review: None,
            invocation: None,
        }));
    }

    fn diagnostic(&mut self, text: String) {
        self.diagnostics.push(text);
        if self.diagnostics.len() > MAX_DIAGNOSTICS {
            self.diagnostics.remove(0);
        }
    }
}

fn reset_partial(partial: &mut PartialContent, kind: PartialKind) -> bool {
    let changed = partial.kind != kind
        || !partial.label.is_empty()
        || !partial.value.is_empty()
        || !partial.chunks.is_empty();
    partial.kind = kind;
    partial.label.clear();
    partial.value.clear();
    partial.chunks = Arc::default();
    changed
}

fn append_delta(partial: &mut PartialContent, delta: &Value) -> bool {
    let text = delta
        .get("delta")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let freezes_tail = matches!(partial.kind, PartialKind::Text | PartialKind::Thinking)
        && partial.value.len() > STREAM_TAIL_MAX_BYTES;
    let changed = !text.is_empty() || freezes_tail;
    partial.value.push_str(text);
    if matches!(partial.kind, PartialKind::Text | PartialKind::Thinking) {
        freeze_stream_chunks(partial);
    }
    changed
}

fn freeze_stream_chunks(partial: &mut PartialContent) {
    while partial.value.len() > STREAM_TAIL_MAX_BYTES {
        let mut split = STREAM_CHUNK_BYTES.min(partial.value.len());
        while !partial.value.is_char_boundary(split) {
            split -= 1;
        }
        if let Some(newline) = partial.value[..split].rfind('\n')
            && newline > STREAM_CHUNK_BYTES / 2
        {
            split = newline + 1;
        }
        let chunk = partial.value[..split].to_owned();
        partial.value.drain(..split);
        Arc::make_mut(&mut partial.chunks).push(Arc::from(chunk));
    }
}

fn finish_content(partial: &mut PartialContent, delta: &Value) -> bool {
    let Some(content) = delta.get("content").and_then(Value::as_str) else {
        return false;
    };
    let changed = partial.value != content || !partial.chunks.is_empty();
    if changed {
        partial.value = content.to_owned();
        partial.chunks = Arc::default();
    }
    changed
}

fn cache_usage(message: &Value) -> Option<(u64, u64)> {
    let usage = message.get("usage")?;
    let input = usage.get("input").and_then(Value::as_u64)?;
    let cache_read = usage.get("cacheRead").and_then(Value::as_u64).unwrap_or(0);
    let cache_write = usage.get("cacheWrite").and_then(Value::as_u64).unwrap_or(0);
    let prompt_tokens = input.saturating_add(cache_read).saturating_add(cache_write);
    (prompt_tokens > 0).then_some((cache_read, prompt_tokens))
}

fn retry_notice(event: &Value) -> String {
    let kind = text_field(event, "type");
    let attempt = event.get("attempt").and_then(Value::as_u64);
    attempt.map_or(kind.clone(), |attempt| {
        format!("{kind} · attempt {attempt}")
    })
}
