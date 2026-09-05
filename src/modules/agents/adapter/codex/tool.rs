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

pub(super) fn is_tool_kind(kind: &str) -> bool {
    matches!(
        kind,
        "commandExecution"
            | "mcpToolCall"
            | "fileChange"
            | "webSearch"
            | "dynamicToolCall"
            | "collabAgentToolCall"
            | "imageView"
            | "imageGeneration"
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
        "imageView" => (
            "view_image".into(),
            json!({"path": item.get("path").cloned().unwrap_or(Value::Null)}),
        ),
        "imageGeneration" => (
            "image_generation".into(),
            item.get("arguments").cloned().unwrap_or_else(|| json!({})),
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
        "collabAgentToolCall" => ToolCategory::Delegate,
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
        "imageView" => "View image",
        "imageGeneration" => "Generate image",
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
mod tests {
    use super::*;

    #[test]
    fn command_projection_preserves_native_actions_and_targets() {
        let item = json!({
            "type":"commandExecution",
            "id":"command-1",
            "command":"cat one && rg needle two",
            "commandActions":[
                {"type":"read","path":"one","name":"Read one"},
                {"type":"search","path":"two","query":"needle"}
            ],
            "status":"inProgress"
        });
        let projection = project(&item, "commandExecution");
        assert_eq!(projection.name, "bash");
        assert_eq!(projection.args["command"], item["command"]);
        assert_eq!(projection.args["commandActions"], item["commandActions"]);
        assert_eq!(projection.metadata.category, Some(ToolCategory::Execute));
        assert_eq!(projection.metadata.targets, ["one", "two"]);
        assert_eq!(projection.metadata.native, Some(item));
    }

    #[test]
    fn homogeneous_and_unknown_command_actions_keep_native_categories() {
        let reads = json!({
            "commandActions":[
                {"type":"read","path":"one"},
                {"type":"read","path":"two"}
            ]
        });
        assert_eq!(
            metadata(&reads, "commandExecution").category,
            Some(ToolCategory::Read)
        );
        let unknown = json!({"commandActions":[{"type":"unknown","command":"pwd"}]});
        assert_eq!(
            metadata(&unknown, "commandExecution").category,
            Some(ToolCategory::Execute)
        );
    }

    #[test]
    fn file_change_metadata_keeps_every_target() {
        let item = json!({
            "type":"fileChange",
            "changes":[{"path":"a.rs"},{"path":"b.rs"}]
        });
        let metadata = metadata(&item, "fileChange");
        assert_eq!(metadata.category, Some(ToolCategory::Change));
        assert_eq!(metadata.targets, ["a.rs", "b.rs"]);
        assert_eq!(metadata.native, Some(item));
    }
}
