use super::*;

impl ConversationState {
    pub(super) fn tool_index(&self, id: &str) -> Option<usize> {
        self.items.rposition(|item| {
            item.kind == TranscriptKind::Tool && item.tool_call_id.as_deref() == Some(id)
        })
    }

    pub(super) fn start_tool(&mut self, event: &Value) {
        let id = text_field(event, "toolCallId");
        let name = text_field(event, "toolName");
        let args_value = event.get("args");
        let mut details = ToolDetails::from_call(&name, args_value, event.get("toolMetadata"));
        details.state = ToolExecutionState::Running;
        let presentation = args_value.and_then(|args| tool_presentation(&name, args));
        let args = args_value
            .map(|args| format_tool_arguments(&name, args))
            .unwrap_or_default();
        if let Some(index) = self.tool_index(&id) {
            let mut item = self.items[index].clone();
            let value = Arc::make_mut(&mut item);
            value.label = display_tool_name(&name);
            value.text = args;
            value.tool_presentation = presentation;
            if let Some(previous) = &value.tool_details {
                details.result.clone_from(&previous.result);
                if event.get("toolMetadata").is_none() {
                    details.metadata.clone_from(&previous.metadata);
                }
            }
            value.tool_details = Some(Arc::new(details));
            value.streaming = true;
            self.items.set(index, item);
            self.tools.insert(id, index);
            return;
        }
        self.items.push(Arc::new(TranscriptItem {
            kind: TranscriptKind::Tool,
            label: display_tool_name(&name),
            text: args,
            images: Arc::default(),
            files: Arc::default(),
            stream_chunks: Arc::default(),
            streaming: true,
            is_error: false,
            tool_call_id: Some(id.clone()),
            tool_output: String::new(),
            tool_presentation: presentation,
            tool_details: Some(Arc::new(details)),
            tool_review: None,
            invocation: None,
        }));
        self.tools.insert(id, self.items.len() - 1);
    }

    pub(super) fn update_tool_metadata(&mut self, event: &Value) -> bool {
        let id = text_field(event, "toolCallId");
        let Some(index) = self.tool_index(&id) else {
            return false;
        };
        let mut item = self.items[index].clone();
        let value = Arc::make_mut(&mut item);
        let Some(details) = value.tool_details.as_mut().map(Arc::make_mut) else {
            return false;
        };
        if let Some(metadata) = event
            .get("toolMetadata")
            .and_then(|metadata| serde_json::from_value(metadata.clone()).ok())
        {
            details.metadata = metadata;
        }
        if let Some(args) = event.get("args") {
            details.arguments = args.clone();
            value.text = format_tool_arguments(&details.name, args);
            value.tool_presentation = tool_presentation(&details.name, args);
        }
        if *self.items[index] == *item {
            return false;
        }
        self.items.set(index, item);
        true
    }

    pub(super) fn update_tool(&mut self, event: &Value) -> bool {
        let id = text_field(event, "toolCallId");
        let Some(index) = self.tools.get(&id).copied() else {
            return false;
        };
        let output = event
            .get("partialResult")
            .map(result_text)
            .unwrap_or_default();
        let Some(item) = self.items.get(index) else {
            return false;
        };
        if item.tool_output == output
            && item
                .tool_details
                .as_ref()
                .and_then(|details| details.result.as_ref())
                == event.get("partialResult")
        {
            return false;
        }
        let mut item = item.clone();
        let value = Arc::make_mut(&mut item);
        value.tool_output = output;
        if let Some(details) = value.tool_details.as_mut().map(Arc::make_mut) {
            details.result = event.get("partialResult").cloned();
        }
        self.items.set(index, item);
        true
    }

    pub(super) fn end_tool(&mut self, event: &Value) {
        let id = text_field(event, "toolCallId");
        let is_error = event
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(index) = self.tools.remove(&id)
            && let Some(item) = self.items.get(index)
        {
            let mut item = item.clone();
            let value = Arc::make_mut(&mut item);
            if let Some(result) = event.get("result") {
                apply_tool_result(value, result, false);
            }
            value.streaming = false;
            value.is_error = is_error;
            if let Some(details) = value.tool_details.as_mut().map(Arc::make_mut) {
                details.state = if is_error {
                    ToolExecutionState::Failed
                } else {
                    ToolExecutionState::Succeeded
                };
            }
            self.items.set(index, item);
        }
    }

    pub(super) fn review_tool(&mut self, event: &Value) {
        let id = text_field(event, "toolCallId");
        let state = match event.get("state").and_then(Value::as_str) {
            Some("reviewing") => ToolReviewState::Reviewing,
            Some("approved") => ToolReviewState::Approved,
            Some("blocked") => ToolReviewState::Blocked,
            _ => return,
        };
        let index = self
            .tools
            .get(&id)
            .copied()
            .or_else(|| self.tool_index(&id));
        let Some(index) = index else {
            return;
        };
        let Some(item) = self.items.get(index) else {
            return;
        };
        let mut item = item.clone();
        Arc::make_mut(&mut item).tool_review = Some(ToolReview {
            state,
            detail: event
                .get("detail")
                .and_then(Value::as_str)
                .map(str::to_owned),
        });
        self.items.set(index, item);
    }
}

pub(super) fn tool_name(value: &Value) -> Option<&str> {
    value
        .get("name")
        .or_else(|| value.get("toolName"))
        .and_then(Value::as_str)
}

pub(super) fn tool_arguments(value: &Value) -> String {
    value
        .get("arguments")
        .map_or_else(String::new, |arguments| {
            format_tool_arguments(tool_name(value).unwrap_or_default(), arguments)
        })
}

fn format_tool_arguments(name: &str, arguments: &Value) -> String {
    if name == "request_user_input"
        && let Some(script) = arguments.get("script").and_then(Value::as_str)
        && !script.is_empty()
    {
        let question = arguments
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        return if question.is_empty() {
            format!("{COMMAND_BLOCK_PREFIX}{script}")
        } else {
            format!("{question}\n\n{COMMAND_BLOCK_PREFIX}{script}")
        };
    }
    readable_json(arguments)
}

const COMMAND_BLOCK_MARKER: &str = "\n\nCommand:\n";
const COMMAND_BLOCK_PREFIX: &str = "Command:\n";

pub(crate) fn split_command_block(text: &str) -> Option<(&str, &str)> {
    if let Some(index) = text.rfind(COMMAND_BLOCK_MARKER) {
        return Some((&text[..index], &text[index + COMMAND_BLOCK_MARKER.len()..]));
    }
    text.strip_prefix(COMMAND_BLOCK_PREFIX)
        .map(|command| ("", command))
}

pub(super) fn tool_presentation(name: &str, arguments: &Value) -> Option<ToolPresentation> {
    let path = arguments
        .get("path")
        .or_else(|| arguments.get("file_path"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    match CommonTool::from_name(name) {
        Some(CommonTool::Edit) => {
            Some(ToolPresentation::edit(path, preview_edit_counts(arguments)))
        }
        Some(CommonTool::Write) => arguments
            .get("content")
            .and_then(Value::as_str)
            .map(|content| ToolPresentation::write(path, content)),
        Some(CommonTool::Read | CommonTool::Bash) | None => None,
    }
}

fn preview_edit_counts(arguments: &Value) -> (usize, usize) {
    if let Some(diff) = arguments.get("diff").and_then(Value::as_str) {
        return change_counts(diff);
    }
    if let Some(changes) = arguments.get("changes").and_then(Value::as_array) {
        return changes.iter().fold((0usize, 0usize), |counts, change| {
            let (additions, deletions) = change
                .get("diff")
                .and_then(Value::as_str)
                .map(change_counts)
                .unwrap_or_default();
            (
                counts.0.saturating_add(additions),
                counts.1.saturating_add(deletions),
            )
        });
    }
    let edits = arguments.get("edits").and_then(Value::as_array);
    let legacy = arguments
        .get("oldText")
        .and_then(Value::as_str)
        .zip(arguments.get("newText").and_then(Value::as_str));
    if let Some(edits) = edits {
        return edit_pair_counts(edits.iter().filter_map(|edit| {
            edit.get("oldText")
                .and_then(Value::as_str)
                .zip(edit.get("newText").and_then(Value::as_str))
        }));
    }
    edit_pair_counts(legacy)
}

fn edit_pair_counts<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> (usize, usize) {
    pairs
        .into_iter()
        .fold((0, 0), |(additions, deletions), (old, new)| {
            (
                additions.saturating_add(new.lines().count()),
                deletions.saturating_add(old.lines().count()),
            )
        })
}

pub(super) fn apply_tool_result(item: &mut TranscriptItem, result: &Value, message: bool) {
    item.tool_output = if message {
        message_text(result)
    } else {
        result_text(result)
    };
    item.is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    item.streaming = false;
    if let Some(details) = item.tool_details.as_mut().map(Arc::make_mut) {
        details.result = Some(result.clone());
        details.state = if item.is_error {
            ToolExecutionState::Failed
        } else {
            ToolExecutionState::Succeeded
        };
    }
    let details = result.get("details");
    if let Some(diff) = details
        .and_then(|details| details.get("diff"))
        .and_then(Value::as_str)
        .filter(|diff| !diff.is_empty())
        && let Some(presentation) = item.tool_presentation.as_mut()
    {
        let first_changed_line = details
            .and_then(|details| details.get("firstChangedLine"))
            .and_then(Value::as_u64);
        presentation.apply_edit_result(diff, first_changed_line);
    }
}

pub(crate) fn display_tool_name(name: &str) -> String {
    let title = name
        .trim()
        .split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        "Tool".into()
    } else {
        title
    }
}

fn result_text(value: &Value) -> String {
    value
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| readable_json(value))
}

fn readable_json(value: &Value) -> String {
    if let Some(raw) = value.as_str()
        && let Ok(parsed) = serde_json::from_str::<Value>(raw)
    {
        return readable_json(&parsed);
    }
    let mut output = String::new();
    write_readable_json(value, 0, &mut output);
    if output.is_empty() {
        "None".into()
    } else {
        output
    }
}

fn write_readable_json(value: &Value, depth: usize, output: &mut String) {
    match value {
        Value::Object(fields) if fields.is_empty() => output.push_str("None"),
        Value::Object(fields) => {
            for (index, (key, value)) in fields.iter().enumerate() {
                if index > 0 {
                    output.push('\n');
                }
                push_indent(output, depth);
                output.push_str(&display_field_name(key));
                output.push(':');
                if let Some(value) = readable_scalar(value) {
                    output.push(' ');
                    output.push_str(&value);
                } else {
                    output.push('\n');
                    write_readable_json(value, depth.saturating_add(1), output);
                }
            }
        }
        Value::Array(items) if items.is_empty() => output.push_str("None"),
        Value::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                if index > 0 {
                    output.push('\n');
                }
                push_indent(output, depth);
                output.push('-');
                if let Some(value) = readable_scalar(value) {
                    output.push(' ');
                    output.push_str(&value);
                } else {
                    output.push('\n');
                    write_readable_json(value, depth.saturating_add(1), output);
                }
            }
        }
        value => output.push_str(&readable_scalar(value).unwrap_or_default()),
    }
}

fn readable_scalar(value: &Value) -> Option<String> {
    match value {
        Value::Null => Some("None".into()),
        Value::Bool(value) => Some(if *value { "Yes" } else { "No" }.into()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => Some(value.clone()),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn push_indent(output: &mut String, depth: usize) {
    output.extend(std::iter::repeat_n(' ', depth.saturating_mul(2)));
}

fn display_field_name(name: &str) -> String {
    let mut words = String::with_capacity(name.len());
    let mut previous_was_lowercase = false;
    for character in name.chars() {
        if character == '_' {
            if !words.ends_with(' ') {
                words.push(' ');
            }
            previous_was_lowercase = false;
        } else {
            if character.is_uppercase() && previous_was_lowercase {
                words.push(' ');
            }
            words.extend(character.to_lowercase());
            previous_was_lowercase = character.is_lowercase();
        }
    }
    let mut characters = words.trim().chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
}
