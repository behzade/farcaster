use serde_json::{Value, json};

use crate::agents::{CommonTool, ToolCategory, ToolMetadata};

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
            let changes = item.get("changes").cloned().unwrap_or_else(|| json!([]));
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
        native: Some(item.clone()),
    }
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

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;
