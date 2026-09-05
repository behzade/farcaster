use serde_json::Value;

use crate::agents::{CommonTool, ToolCategory, ToolMetadata};

pub(super) fn normalize_opencode_tool(name: &str, arguments: &Value) -> (String, Value) {
    let normalized_name = name.trim().to_ascii_lowercase();
    let common = CommonTool::from_name(&normalized_name).or(match normalized_name.as_str() {
        "read_file" => Some(CommonTool::Read),
        "write_file" => Some(CommonTool::Write),
        "edit_file" | "apply_patch" | "patch" => Some(CommonTool::Edit),
        "shell" | "command" | "terminal" => Some(CommonTool::Bash),
        _ => None,
    });
    let canonical = common
        .map(|tool| tool.name())
        .or(match normalized_name.as_str() {
            "glob" => Some("find"),
            "list" | "list_files" => Some("ls"),
            "webfetch" | "fetch" => Some("web_fetch"),
            "websearch" => Some("web_search"),
            _ => None,
        });
    let Some(canonical) = canonical else {
        return (name.to_owned(), arguments.clone());
    };
    let mut normalized = arguments.as_object().cloned().unwrap_or_default();
    rename_argument(&mut normalized, "path", &["file_path", "filePath"]);
    if common == Some(CommonTool::Edit) {
        rename_argument(&mut normalized, "oldText", &["old_string", "oldString"]);
        rename_argument(&mut normalized, "newText", &["new_string", "newString"]);
    } else if common == Some(CommonTool::Bash) {
        rename_argument(&mut normalized, "command", &["cmd"]);
    }
    (canonical.into(), Value::Object(normalized))
}

pub(super) fn opencode_tool_metadata(name: &str, arguments: &Value, native: Value) -> ToolMetadata {
    let normalized = name.trim().to_ascii_lowercase();
    let (category, verb, keys): (ToolCategory, Option<&str>, &[&str]) = match normalized.as_str() {
        "read" | "read_file" => (ToolCategory::Read, Some("Read"), &["path"]),
        "grep" | "search" | "rg" | "glob" | "find" => {
            (ToolCategory::Search, Some("Search"), &["path", "directory"])
        }
        "ls" | "list" | "list_files" => (ToolCategory::List, Some("List"), &["path", "directory"]),
        "write" | "write_file" => (ToolCategory::Change, Some("Write"), &["path"]),
        "edit" | "edit_file" | "apply_patch" | "patch" => {
            (ToolCategory::Change, Some("Edit"), &["path"])
        }
        "bash" | "shell" | "command" | "terminal" => {
            (ToolCategory::Execute, Some("Run command"), &[])
        }
        "webfetch" | "web_fetch" | "fetch" => (ToolCategory::Fetch, Some("Fetch"), &["url"]),
        "websearch" | "web_search" => (ToolCategory::Fetch, Some("Search web"), &["url"]),
        "task" | "agent" | "delegate" => (ToolCategory::Delegate, Some("Delegate"), &[]),
        _ => (ToolCategory::Other, None, &[]),
    };
    let targets = keys
        .iter()
        .filter_map(|key| arguments.get(*key).and_then(Value::as_str))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let native_title = native
        .get("title")
        .or_else(|| native.pointer("/metadata/title"))
        .or_else(|| native.pointer("/state/title"))
        .or_else(|| native.pointer("/state/metadata/title"))
        .and_then(Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .map(str::to_owned);
    let title = native_title.or_else(|| {
        verb.map(|verb| match targets.first() {
            Some(target) => format!("{verb} {target}"),
            None => verb.to_owned(),
        })
    });
    ToolMetadata {
        category: Some(category),
        title,
        targets,
        native: Some(native),
    }
}

fn rename_argument(
    arguments: &mut serde_json::Map<String, Value>,
    canonical: &str,
    aliases: &[&str],
) {
    let value = aliases.iter().find_map(|alias| arguments.remove(*alias));
    if !arguments.contains_key(canonical)
        && let Some(value) = value
    {
        arguments.insert(canonical.into(), value);
    }
    for alias in aliases {
        arguments.remove(*alias);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn common_tools_use_shared_names_and_arguments() {
        for (source, canonical) in [
            ("read_file", "read"),
            ("write_file", "write"),
            ("apply_patch", "edit"),
            ("shell", "bash"),
            ("glob", "find"),
            ("list", "ls"),
            ("webfetch", "web_fetch"),
        ] {
            assert_eq!(normalize_opencode_tool(source, &json!({})).0, canonical);
        }
        assert_eq!(
            normalize_opencode_tool(
                "edit",
                &json!({"filePath": "src/main.rs", "oldString": "old", "newString": "new"}),
            )
            .1,
            json!({"path": "src/main.rs", "oldText": "old", "newText": "new"})
        );
    }

    #[test]
    fn metadata_keeps_native_input_and_does_not_guess_custom_intent() {
        let native = json!({
            "state": {
                "input": {"filePath": "src/main.rs"},
                "metadata": {"title": "Inspect source"}
            }
        });
        let metadata =
            opencode_tool_metadata("read", &json!({"path": "src/main.rs"}), native.clone());
        assert_eq!(metadata.category, Some(ToolCategory::Read));
        assert_eq!(metadata.title.as_deref(), Some("Inspect source"));
        assert_eq!(metadata.targets, ["src/main.rs"]);
        assert_eq!(metadata.native, Some(native));

        let custom = opencode_tool_metadata(
            "mcp_database",
            &json!({"path": "not-a-file-fact"}),
            json!({"progress": 1}),
        );
        assert_eq!(custom.category, Some(ToolCategory::Other));
        assert!(custom.title.is_none());
        assert!(custom.targets.is_empty());
    }
}
