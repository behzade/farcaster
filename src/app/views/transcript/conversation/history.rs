use super::*;

impl ConversationState {
    pub(crate) fn replace_history(&mut self, messages: &[Value]) {
        self.items.clear();
        self.average_cache_hit_rate = None;
        self.cache_read_tokens = 0;
        self.prompt_tokens = 0;
        for message in messages {
            if message.get("role").and_then(Value::as_str) == Some("assistant") {
                self.record_cache_hit_rate(message);
            }
            self.project_history_message(message);
        }
        self.live_message = None;
        self.content.clear();
        self.dirty_content.clear();
        self.projected_content.clear();
        self.tools.clear();
        self.optimistic_user = None;
    }

    fn project_history_message(&mut self, message: &Value) {
        if message.get("role").and_then(Value::as_str) == Some("toolResult") {
            let id = message
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !id.is_empty()
                && let Some(index) = self.items.rposition(|item| {
                    item.kind == TranscriptKind::Tool && item.tool_call_id.as_deref() == Some(id)
                })
            {
                let mut item = self.items[index].clone();
                apply_tool_result(Arc::make_mut(&mut item), message, true);
                self.items.set(index, item);
                return;
            }
        }
        let mut projected = project_message_items(message);
        if message.get("role").and_then(Value::as_str) == Some("toolResult") {
            for item in &mut projected {
                item.tool_output = std::mem::take(&mut item.text);
            }
        }
        self.items.extend(projected.into_iter().map(Arc::new));
    }
}

pub(super) fn peer_transcript_item(peer: PeerMessage) -> TranscriptItem {
    TranscriptItem {
        kind: TranscriptKind::PeerMessage,
        label: format!("Worker · {}", peer.from),
        text: peer.message,
        images: Arc::default(),
        stream_chunks: Arc::default(),
        streaming: false,
        is_error: false,
        tool_call_id: None,
        tool_output: String::new(),
        tool_presentation: None,
        tool_review: None,
        invocation: None,
    }
}

pub(super) fn project_message_items(message: &Value) -> Vec<TranscriptItem> {
    let Some(role) = message.get("role").and_then(Value::as_str) else {
        return Vec::new();
    };
    let is_error = message
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || message.get("stopReason").and_then(Value::as_str) == Some("error");
    if role == "assistant" {
        let mut items: Vec<TranscriptItem> = message
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|block| {
                        let (kind, label, text) = match block.get("type").and_then(Value::as_str) {
                            Some("text") => (
                                TranscriptKind::Assistant,
                                String::new(),
                                block.get("text").and_then(Value::as_str)?.to_owned(),
                            ),
                            Some("thinking") => {
                                let thinking = block
                                    .get("thinking")
                                    .and_then(Value::as_str)
                                    .filter(|thinking| !thinking.trim().is_empty())?;
                                (TranscriptKind::Thinking, String::new(), thinking.to_owned())
                            }
                            Some("toolCall") => (
                                TranscriptKind::Tool,
                                tool_name(block)
                                    .map(display_tool_name)
                                    .unwrap_or_else(|| "Tool".into()),
                                tool_arguments(block),
                            ),
                            _ => return None,
                        };
                        Some(TranscriptItem {
                            kind,
                            label,
                            text,
                            images: Arc::default(),
                            stream_chunks: Arc::default(),
                            streaming: false,
                            is_error: false,
                            tool_call_id: block
                                .get("id")
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                            tool_output: String::new(),
                            tool_presentation: tool_name(block).and_then(|name| {
                                block
                                    .get("arguments")
                                    .and_then(|arguments| tool_presentation(name, arguments))
                            }),
                            tool_review: None,
                            invocation: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        if is_error && let Some(error) = assistant_error_text(message, !items.is_empty()) {
            items.push(model_error_item(error));
        }
        return items;
    }
    let projected_user = (role == "user").then(|| projected_user_message_text(message));
    if let Some(peer) = projected_user.as_deref().and_then(PeerMessage::from_prompt) {
        return vec![peer_transcript_item(peer)];
    }
    let (kind, label, display) = match role {
        "user" => (TranscriptKind::User, String::new(), true),
        "toolResult" => (
            TranscriptKind::Tool,
            message
                .get("toolName")
                .and_then(Value::as_str)
                .map(display_tool_name)
                .unwrap_or_else(|| "Tool".into()),
            true,
        ),
        "bashExecution" => (TranscriptKind::Tool, "Shell".into(), true),
        "custom" => (
            if message.get("customType").and_then(Value::as_str) == Some("subagent-result") {
                TranscriptKind::AgentResult
            } else {
                TranscriptKind::Custom
            },
            if message.get("customType").and_then(Value::as_str) == Some("subagent-result") {
                "Subagent result".into()
            } else {
                "Extension".into()
            },
            message.get("customType").and_then(Value::as_str) != Some("background-job-result")
                && message
                    .get("display")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
        ),
        "branchSummary" | "compactionSummary" => (TranscriptKind::Notice, "Summary".into(), true),
        _ => return Vec::new(),
    };
    if !display {
        return Vec::new();
    }
    let invocation = if kind == TranscriptKind::User {
        message
            .get("farcasterInvocationResolution")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                message
                    .get("piUserInvocation")
                    .and_then(Value::as_str)
                    .map(|_| message_text(message))
            })
    } else {
        None
    };
    vec![TranscriptItem {
        kind: if is_error && kind != TranscriptKind::Tool {
            TranscriptKind::Error
        } else {
            kind
        },
        label,
        text: if let Some(projected_user) = projected_user {
            projected_user
        } else {
            message_text(message)
        },
        images: if kind == TranscriptKind::User {
            message_images(message)
        } else {
            Arc::default()
        },
        stream_chunks: Arc::default(),
        streaming: false,
        is_error,
        tool_call_id: message
            .get("toolCallId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        tool_output: String::new(),
        tool_presentation: None,
        tool_review: None,
        invocation,
    }]
}

pub(super) fn message_text(message: &Value) -> String {
    if let Some(content) = message.get("content")
        && let Some(text) = content.as_str()
    {
        return text.to_owned();
    }
    if let Some(blocks) = message.get("content").and_then(Value::as_array) {
        let text = blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        let has_image = blocks
            .iter()
            .any(|block| block.get("type").and_then(Value::as_str) == Some("image"));
        if !text.is_empty() || has_image {
            return text;
        }
    }
    message
        .get("summary")
        .and_then(Value::as_str)
        .or_else(|| message.get("output").and_then(Value::as_str))
        .unwrap_or_default()
        .to_owned()
}

fn projected_user_message_text(message: &Value) -> String {
    let text = message
        .get("farcasterUserInvocation")
        .and_then(Value::as_str)
        .or_else(|| message.get("piUserInvocation").and_then(Value::as_str))
        .map_or_else(|| message_text(message), str::to_owned);
    pasted_file_summary(&text).to_owned()
}

pub(crate) fn annotate_prompt_presentations(
    messages: &mut [Value],
    presentations: &[crate::agents::PromptPresentation],
) {
    let mut cursor = 0;
    for presentation in presentations {
        let Some((offset, message)) =
            messages[cursor..]
                .iter_mut()
                .enumerate()
                .find(|(_, message)| {
                    message.get("role").and_then(Value::as_str) == Some("user")
                        && message_text(message) == presentation.resolved_message
                })
        else {
            continue;
        };
        cursor += offset + 1;
        let Some(message) = message.as_object_mut() else {
            continue;
        };
        message.insert(
            "farcasterUserInvocation".into(),
            presentation.display_message.clone().into(),
        );
        message.insert(
            "farcasterInvocationResolution".into(),
            presentation.invocation.clone().into(),
        );
    }
}

pub(super) fn user_message_text(message: &str, image_count: usize) -> String {
    let message = pasted_file_summary(message);
    if image_count == 0 {
        return message.to_owned();
    }
    let attachment = if image_count == 1 {
        "Attached image".to_owned()
    } else {
        format!("Attached {image_count} images")
    };
    if message.is_empty() {
        attachment
    } else {
        format!("{message}\n\n{attachment}")
    }
}

pub(super) fn pasted_file_summary(message: &str) -> &str {
    message
        .split_once("\n\n--- BEGIN PASTED FILE ")
        .map_or(message, |(summary, _)| summary)
}

pub(super) fn decode_prompt_images(images: &[PromptImage]) -> Arc<Vec<Arc<Image>>> {
    decode_images(
        images
            .iter()
            .map(|image| (image.data.as_str(), image.mime_type.as_str())),
    )
}

fn message_images(message: &Value) -> Arc<Vec<Arc<Image>>> {
    decode_images(
        message
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("image"))
            .filter_map(|block| {
                block
                    .get("data")
                    .and_then(Value::as_str)
                    .zip(block.get("mimeType").and_then(Value::as_str))
            }),
    )
}

fn decode_images<'a>(images: impl IntoIterator<Item = (&'a str, &'a str)>) -> Arc<Vec<Arc<Image>>> {
    Arc::new(
        images
            .into_iter()
            .filter_map(|(data, mime_type)| decode_image(data, mime_type))
            .collect(),
    )
}

fn decode_image(data: &str, mime_type: &str) -> Option<Arc<Image>> {
    let format = ImageFormat::from_mime_type(mime_type)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()?;
    (!bytes.is_empty()).then(|| Arc::new(Image::from_bytes(format, bytes)))
}

fn assistant_error_text(message: &Value, has_content: bool) -> Option<String> {
    message
        .get("errorMessage")
        .and_then(Value::as_str)
        .filter(|error| !error.is_empty())
        .map(str::to_owned)
        .or_else(|| has_content.then(|| "Unknown error".into()))
}

fn model_error_item(text: String) -> TranscriptItem {
    TranscriptItem {
        kind: TranscriptKind::Error,
        label: "Model error".into(),
        text,
        images: Arc::default(),
        stream_chunks: Arc::default(),
        streaming: false,
        is_error: true,
        tool_call_id: None,
        tool_output: String::new(),
        tool_presentation: None,
        tool_review: None,
        invocation: None,
    }
}
