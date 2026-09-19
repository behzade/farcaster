use serde_json::{Value, json};

use crate::{CommonTool, ToolCategory, ToolMetadata};

fn reconcile_terminal_status(native: &mut Value) {
    let state = native.get("state").unwrap_or(native);
    let Some(status) = state.get("status").and_then(Value::as_str) else {
        return;
    };
    let terminal_status = match status {
        "completed" => "completed",
        "error" | "failed" => {
            let interrupted = state
                .pointer("/error/type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("aborted"));
            if interrupted { "interrupted" } else { "error" }
        }
        _ => return,
    };
    let metadata_status = if native.get("state").is_some() {
        native.pointer_mut("/state/metadata/status")
    } else {
        native.pointer_mut("/metadata/status")
    };
    if let Some(metadata_status) = metadata_status.filter(|value| value.as_str() == Some("running"))
    {
        *metadata_status = Value::String(terminal_status.into());
    }
}

pub(super) fn normalize_opencode_tool(
    name: &str,
    arguments: &Value,
    native: &Value,
) -> (String, Value) {
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
    if matches!(common, Some(CommonTool::Edit | CommonTool::Write))
        && let Some(files) = native
            .pointer("/metadata/files")
            .or_else(|| native.pointer("/state/metadata/files"))
            .and_then(Value::as_array)
    {
        let changes = files
            .iter()
            .filter_map(|file| {
                let path = file.get("file")?.as_str().filter(|path| !path.is_empty())?;
                let diff = file
                    .get("patch")
                    .and_then(Value::as_str)
                    .map(strip_patch_preamble);
                Some(json!({"path": path, "diff": diff}))
            })
            .collect::<Vec<_>>();
        if let Some(first) = changes.first() {
            normalized
                .entry("path")
                .or_insert_with(|| first["path"].clone());
            normalized.insert("changes".into(), Value::Array(changes));
        }
    }
    (canonical.into(), Value::Object(normalized))
}

// OpenCode prefixes unified diffs with an Index line and a separator.
fn strip_patch_preamble(diff: &str) -> &str {
    if let Some((index, rest)) = diff.split_once('\n')
        && index.starts_with("Index: ")
        && let Some((separator, patch)) = rest.split_once('\n')
        && separator.trim_end_matches('\r')
            == "==================================================================="
    {
        return patch;
    }
    diff
}

pub(super) fn opencode_tool_metadata(
    name: &str,
    arguments: &Value,
    mut native: Value,
) -> ToolMetadata {
    reconcile_terminal_status(&mut native);
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
    let mut targets = keys
        .iter()
        .filter_map(|key| arguments.get(*key).and_then(Value::as_str))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if category == ToolCategory::Change
        && let Some(changes) = arguments.get("changes").and_then(Value::as_array)
    {
        for path in changes
            .iter()
            .filter_map(|change| change.get("path").and_then(Value::as_str))
        {
            if !path.is_empty() && !targets.iter().any(|target| target == path) {
                targets.push(path.to_owned());
            }
        }
    }
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
#[path = "tool_tests.rs"]
mod tests;
