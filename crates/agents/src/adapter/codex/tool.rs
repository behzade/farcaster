use serde_json::{Value, json};

use crate::{CommonTool, ToolCategory, ToolMetadata};

pub(super) struct Projection {
    pub(super) name: String,
    pub(super) args: Value,
    pub(super) metadata: ToolMetadata,
}

pub(super) fn project(item: &Value, kind: &str) -> Projection {
    let (name, args) = call(item, kind);
    Projection {
        name,
        args,
        metadata: metadata(item, kind),
    }
}

pub(super) fn subagent_summary(item: &Value) -> String {
    let agent = item
        .get("agentPath")
        .and_then(Value::as_str)
        .unwrap_or("agent");
    let kind = item
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("updated");
    format!("{agent} {kind}")
}

pub(super) fn is_tool_kind(kind: &str) -> bool {
    matches!(
        kind,
        "commandExecution"
            | "mcpToolCall"
            | "fileChange"
            | "webSearch"
            | "dynamicToolCall"
            | "collabAgentToolCall"
            | "subAgentActivity"
            | "imageView"
            | "imageGeneration"
            | "sleep"
    )
}

pub(super) fn call(item: &Value, kind: &str) -> (String, Value) {
    match kind {
        "fileChange" => {
            let mut changes = item.get("changes").cloned().unwrap_or_else(|| json!([]));
            for change in changes.as_array_mut().into_iter().flatten() {
                if change.pointer("/kind/type").and_then(Value::as_str) != Some("add") {
                    continue;
                }
                let Some(content) = change.get("diff").and_then(Value::as_str) else {
                    continue;
                };
                // Codex puts full file content in `diff` for additions.
                let lines = content.split_terminator('\n');
                let count = lines.clone().count();
                let start = usize::from(count > 0);
                let mut diff = format!("@@ -0,0 +{start},{count} @@\n");
                for line in lines {
                    diff.push('+');
                    diff.push_str(line);
                    diff.push('\n');
                }
                if !content.is_empty() && !content.ends_with('\n') {
                    diff.push_str("\\ No newline at end of file\n");
                }
                change["diff"] = Value::String(diff);
            }
            let path = changes
                .as_array()
                .and_then(|changes| changes.first())
                .and_then(|change| change.get("path"))
                .cloned()
                .unwrap_or(Value::String(String::new()));
            (
                CommonTool::Edit.name().into(),
                json!({"path": path, "changes": changes}),
            )
        }
        "commandExecution" => {
            let actions = item
                .get("commandActions")
                .cloned()
                .unwrap_or_else(|| json!([]));
            let read_path = actions
                .as_array()
                .filter(|actions| actions.len() == 1)
                .and_then(|actions| actions.first())
                .filter(|action| action.get("type").and_then(Value::as_str) == Some("read"))
                .and_then(|action| action.get("path"))
                .cloned();
            let mut args = serde_json::Map::new();
            args.insert(
                "command".into(),
                item.get("command").cloned().unwrap_or(Value::Null),
            );
            args.insert("commandActions".into(), actions);
            if let Some(cwd) = item.get("cwd") {
                args.insert("cwd".into(), cwd.clone());
            }
            if let Some(path) = read_path {
                args.insert("path".into(), path);
                (CommonTool::Read.name().into(), Value::Object(args))
            } else {
                (CommonTool::Bash.name().into(), Value::Object(args))
            }
        }
        "webSearch" => (
            "web_search".into(),
            json!({"query": web_search_query(item)}),
        ),
        "sleep" => (
            "wait".into(),
            json!({"durationMs": item.get("durationMs").cloned().unwrap_or(Value::Null)}),
        ),
        "imageView" => (
            "view_image".into(),
            json!({"path": item.get("path").cloned().unwrap_or(Value::Null)}),
        ),
        "imageGeneration" => (
            "image_generation".into(),
            item.get("arguments").cloned().unwrap_or_else(|| json!({})),
        ),
        "subAgentActivity" => (
            "agent_activity".into(),
            json!({
                "agentThreadId": item.get("agentThreadId"),
                "agentPath": item.get("agentPath"),
                "kind": item.get("kind"),
            }),
        ),
        "collabAgentToolCall" => {
            let args = [
                "prompt",
                "model",
                "senderThreadId",
                "receiverThreadIds",
                "agentsStates",
            ]
            .into_iter()
            .filter_map(|field| Some((field.into(), item.get(field)?.clone())))
            .collect();
            (
                item.get("tool")
                    .and_then(Value::as_str)
                    .unwrap_or("collabAgent")
                    .to_owned(),
                Value::Object(args),
            )
        }
        _ => {
            let name = item
                .get("tool")
                .and_then(Value::as_str)
                .or_else(|| item.get("name").and_then(Value::as_str))
                .or_else(|| item.get("server").and_then(Value::as_str))
                .unwrap_or(kind);
            let args = item.get("arguments").cloned().unwrap_or_else(|| json!({}));
            (name.to_owned(), args)
        }
    }
}

pub(super) fn metadata(item: &Value, kind: &str) -> ToolMetadata {
    let category = match kind {
        "commandExecution" | "command" => command_actions_category(item),
        "fileChange" => ToolCategory::Change,
        "webSearch" => ToolCategory::Fetch,
        "collabAgentToolCall" | "subAgentActivity" => ToolCategory::Delegate,
        "imageView" => ToolCategory::Read,
        "imageGeneration" => ToolCategory::Change,
        "mcpToolCall" | "dynamicToolCall" => ToolCategory::Other,
        _ => ToolCategory::Other,
    };
    let targets = match kind {
        "fileChange" => paths(item.get("changes")),
        "commandExecution" => paths(item.get("commandActions")),
        "imageGeneration" => item
            .get("savedPath")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .into_iter()
            .collect(),
        "imageView" => item
            .get("path")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    };
    let title = item
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| !title.is_empty())
        .or_else(|| single_action_title(item, kind))
        .map(str::to_owned)
        .or_else(|| generated_title(item, kind, category, &targets));
    ToolMetadata {
        category: Some(category),
        title,
        targets,
        native: Some(call_metadata(item, kind)),
    }
}

fn call_metadata(item: &Value, kind: &str) -> Value {
    if kind != "mcpToolCall" {
        return item.clone();
    }
    let Some(fields) = item.as_object() else {
        return item.clone();
    };
    let mut native = serde_json::Map::new();
    for (key, value) in fields {
        match key.as_str() {
            "error" if !value.is_null() => {}
            "result" if value.is_object() => {
                if let Some(content) = value.get("content") {
                    native.insert(key.clone(), json!({"content":content}));
                }
            }
            _ => {
                native.insert(key.clone(), value.clone());
            }
        }
    }
    Value::Object(native)
}

fn paths(items: Option<&Value>) -> Vec<String> {
    items
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("path").and_then(Value::as_str))
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect()
}

fn single_action_title<'a>(item: &'a Value, kind: &str) -> Option<&'a str> {
    if kind != "commandExecution" {
        return None;
    }
    match item.get("commandActions")?.as_array()?.as_slice() {
        [action] => action
            .get("name")
            .and_then(Value::as_str)
            .filter(|title| !title.is_empty()),
        _ => None,
    }
}

fn generated_title(
    item: &Value,
    kind: &str,
    category: ToolCategory,
    targets: &[String],
) -> Option<String> {
    let verb = match kind {
        "commandExecution" | "command" => match category {
            ToolCategory::Read => "Read",
            ToolCategory::Search => "Search",
            ToolCategory::List => "List",
            _ => "Run command",
        },
        "fileChange" => "Change",
        "webSearch" => "Search web",
        "collabAgentToolCall" => "Delegate",
        "subAgentActivity" => return Some(subagent_summary(item)),
        "imageView" => "View image",
        "imageGeneration" => "Generate image",
        "sleep" => return Some(format!("Waiting {}", wait_duration(item))),
        "mcpToolCall" | "dynamicToolCall" => {
            return item
                .get("tool")
                .or_else(|| item.get("name"))
                .or_else(|| item.get("server"))
                .and_then(Value::as_str)
                .filter(|title| !title.is_empty())
                .map(str::to_owned);
        }
        _ => return None,
    };
    Some(match targets {
        [] => verb.to_owned(),
        [target] => format!("{verb} {target}"),
        targets => format!("{verb} {} files", targets.len()),
    })
}

fn command_actions_category(item: &Value) -> ToolCategory {
    let mut actions = item
        .get("commandActions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|action| match action.get("type").and_then(Value::as_str) {
            Some("read") => ToolCategory::Read,
            Some("search") => ToolCategory::Search,
            Some("listFiles") => ToolCategory::List,
            _ => ToolCategory::Execute,
        });
    let Some(first) = actions.next() else {
        return ToolCategory::Execute;
    };
    if actions.all(|category| category == first) {
        first
    } else {
        ToolCategory::Execute
    }
}

pub(super) fn wait_duration(item: &Value) -> String {
    let seconds = item
        .get("durationMs")
        .and_then(Value::as_u64)
        .map(|millis| millis / 1000)
        .unwrap_or(0);
    if seconds > 0 && seconds.is_multiple_of(60) {
        format!("{}m", seconds / 60)
    } else {
        format!("{}s", seconds)
    }
}

pub(super) fn web_search_query(item: &Value) -> Option<&str> {
    item.get("query")
        .and_then(Value::as_str)
        .filter(|query| !query.is_empty())
        .or_else(|| {
            let action = item.get("action")?;
            ["query", "url", "pattern"]
                .into_iter()
                .find_map(|field| action.get(field).and_then(Value::as_str))
        })
}

pub(super) fn completion(item: &Value, kind: &str) -> (Value, bool) {
    let failed = item
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| matches!(status, "failed" | "declined"))
        || item["success"] == false
        || item.pointer("/result/isError").and_then(Value::as_bool) == Some(true);
    let result = if kind == "imageGeneration" {
        Value::Array(image_output(item))
    } else if kind == "mcpToolCall" && item.pointer("/result/content").is_some_and(Value::is_array)
    {
        Value::Array(mcp_output(item))
    } else if let Some(parts) = item["contentItems"].as_array() {
        Value::Array(parts.iter().map(dynamic_output_block).collect())
    } else if let Some(content) = item.pointer("/result/content").and_then(Value::as_array) {
        Value::Array(content.clone())
    } else if failed
        && let Some(error) = item
            .pointer("/error/message")
            .and_then(Value::as_str)
            .or_else(|| item.get("error").and_then(Value::as_str))
    {
        json!([{"type": "text", "text": error}])
    } else if let Some(output) = item.get("aggregatedOutput").and_then(Value::as_str) {
        json!([{"type": "text", "text": output}])
    } else if let Some(output) = item
        .get("result")
        .filter(|value| !value.is_null())
        .or_else(|| item.get("output").filter(|value| !value.is_null()))
    {
        json!([{
            "type": "text",
            "text": output.as_str().map(str::to_owned).unwrap_or_else(|| output.to_string()),
        }])
    } else if kind == "commandExecution" && item.get("aggregatedOutput").is_none_or(Value::is_null)
    {
        json!([{"type": "text", "text": ""}])
    } else if kind == "webSearch" {
        json!([{"type": "text", "text": web_search_query(item).unwrap_or_default()}])
    } else if kind == "fileChange" && !failed {
        json!([{"type": "text", "text": "Applied patch"}])
    } else if kind == "imageView" {
        json!([{
            "type": "text",
            "text": item.get("path").and_then(Value::as_str).unwrap_or_default(),
        }])
    } else if kind == "sleep" {
        json!([{
            "type": "text",
            "text": format!("Waited {}", wait_duration(item)),
        }])
    } else if kind == "subAgentActivity" {
        json!([{"type": "text", "text": subagent_summary(item)}])
    } else if kind == "collabAgentToolCall" {
        json!([{
            "type": "text",
            "text": item
                .get("agentsStates")
                .map(Value::to_string)
                .unwrap_or_else(|| item.get("status").map(Value::to_string).unwrap_or_default()),
        }])
    } else {
        zlog::warn!("Codex {kind} completion has no displayable result");
        json!([])
    };
    let mut details = serde_json::Map::new();
    for key in ["exitCode", "durationMs"] {
        if let Some(value) = item.get(key).filter(|value| !value.is_null())
            && !(key == "durationMs" && kind == "sleep")
        {
            details.insert(key.into(), value.clone());
        }
    }
    if kind == "mcpToolCall" {
        let mut native = serde_json::Map::new();
        if let Some(result) = item["result"].as_object() {
            let fields: serde_json::Map<_, _> = result
                .iter()
                .filter(|(field, _)| field.as_str() != "content")
                .map(|(field, value)| (field.clone(), value.clone()))
                .collect();
            if !fields.is_empty() {
                native.insert("result".into(), Value::Object(fields));
            }
        }
        if let Some(error) = item.get("error").filter(|error| !error.is_null()) {
            native.insert("error".into(), error.clone());
        }
        if !native.is_empty() {
            details.insert("native".into(), Value::Object(native));
        }
    }
    if details.is_empty() {
        (result, failed)
    } else {
        (json!({"content": result, "details": details}), failed)
    }
}

fn dynamic_output_block(block: &Value) -> Value {
    match block["type"].as_str() {
        Some("inputText") if block["text"].is_string() => {
            json!({"type":"text","text":block["text"]})
        }
        Some("inputImage") => block["imageUrl"]
            .as_str()
            .and_then(|url| url.strip_prefix("data:")?.split_once(";base64,"))
            .filter(|(mime, _)| mime.starts_with("image/"))
            .map(|(mime, data)| json!({"type":"image", "mimeType":mime, "data":data}))
            .unwrap_or_else(|| block.clone()),
        _ => block.clone(),
    }
}

pub(super) fn mcp_output(item: &Value) -> Vec<Value> {
    item.pointer("/result/content")
        .and_then(Value::as_array)
        .map(|blocks| farcaster_agent_protocol::normalize_tool_output(blocks).into_owned())
        .unwrap_or_default()
}

pub(super) fn image_output(item: &Value) -> Vec<Value> {
    use base64::Engine as _;
    let mut blocks = Vec::new();
    let saved = item
        .get("savedPath")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty());
    let inline = item
        .get("result")
        .and_then(Value::as_str)
        .filter(|data| !data.is_empty());
    let bytes = inline
        .and_then(|data| {
            let data = data
                .strip_prefix("data:")
                .and_then(|s| s.split_once(";base64,").map(|(_, data)| data))
                .unwrap_or(data);
            (data.len() <= 45 * 1024 * 1024)
                .then(|| base64::engine::general_purpose::STANDARD.decode(data).ok())
                .flatten()
        })
        .or_else(|| {
            saved.and_then(|path| {
                (std::fs::metadata(path).ok()?.len() <= 32 * 1024 * 1024)
                    .then(|| std::fs::read(path).ok())
                    .flatten()
            })
        });
    if let Some(bytes) = bytes {
        let mime = if bytes.starts_with(b"\x89PNG") {
            Some("image/png")
        } else if bytes.starts_with(b"\xff\xd8\xff") {
            Some("image/jpeg")
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            Some("image/webp")
        } else {
            None
        };
        if let Some(mime) = mime {
            blocks.push(json!({"type":"image", "mimeType":mime, "data":base64::engine::general_purpose::STANDARD.encode(bytes)}));
        }
    }
    if let Some(path) = saved {
        blocks.push(json!({"type":"resource_link", "name":std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy(), "uri":path}));
    }
    if blocks.is_empty() {
        let text = if let Some(failure) = item.get("failure").filter(|v| !v.is_null()) {
            format!("Image generation failed: {failure}")
        } else if let Some(id) = item.get("fileId").and_then(Value::as_str) {
            format!(
                "Generated image {id} is stored by the provider, but no local file or image bytes are available."
            )
        } else {
            "Image generation returned no previewable image.".to_owned()
        };
        blocks.push(json!({"type":"text", "text":text}));
    }
    blocks
}

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;
