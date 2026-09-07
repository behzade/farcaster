use serde_json::{Value, json};

use super::AcpProfile;
use crate::agents::{CommonTool, TokenUsage, ToolCategory, ToolMetadata, WorkerUsage};

#[derive(Clone, Default)]
pub(super) struct ConfigIds {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub mode: Option<String>,
}

pub(super) fn metadata_from_session(
    profile: &AcpProfile,
    response: &Value,
) -> (super::super::main_session::MainSessionMetadata, ConfigIds) {
    let options = response
        .get("configOptions")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let (mut metadata, ids) = metadata_from_options(profile, options);
    if metadata.modes.is_empty()
        && let Some(modes) = response
            .pointer("/modes/availableModes")
            .and_then(Value::as_array)
    {
        metadata.modes = modes
            .iter()
            .filter_map(|mode| {
                let id = mode.get("id")?.as_str()?;
                Some(json!({
                    "id": id,
                    "name": mode.get("name").and_then(Value::as_str).unwrap_or(id),
                    "description": mode.get("description").cloned(),
                }))
            })
            .collect();
        if let Some(current) = response
            .pointer("/modes/currentModeId")
            .and_then(Value::as_str)
            && let Some(index) = metadata
                .modes
                .iter()
                .position(|mode| mode.get("id").and_then(Value::as_str) == Some(current))
        {
            metadata.modes.swap(0, index);
        }
    }
    (metadata, ids)
}

pub(super) fn metadata_from_options(
    profile: &AcpProfile,
    options: &[Value],
) -> (super::super::main_session::MainSessionMetadata, ConfigIds) {
    let mut metadata = super::super::main_session::MainSessionMetadata::default();
    let mut ids = ConfigIds::default();
    for option in options {
        let category = option.get("category").and_then(Value::as_str).unwrap_or("");
        let id = option.get("id").and_then(Value::as_str).unwrap_or("");
        let values = option.get("options").and_then(Value::as_array);
        if category == "model" || id == "model" {
            ids.model = Some(id.into());
            metadata.models = values
                .into_iter()
                .flatten()
                .filter_map(|value| {
                    let id = value.get("value")?.as_str()?;
                    Some(json!({
                        "id": id,
                        "name": value.get("name").and_then(Value::as_str).unwrap_or(id),
                        "provider": profile.backend,
                        "contextWindow": 0,
                        "reasoning": true,
                    }))
                })
                .collect();
        } else if category == "mode" || id == "mode" {
            ids.mode = Some(id.into());
            metadata.modes = values
                .into_iter()
                .flatten()
                .filter_map(|value| {
                    let id = value.get("value")?.as_str()?;
                    Some(json!({
                        "id": id,
                        "name": value.get("name").and_then(Value::as_str).unwrap_or(id),
                        "description": value.get("description").cloned(),
                    }))
                })
                .collect();
        } else if category == "thought_level"
            || category == "reasoning"
            || id.contains("effort")
            || id.contains("reasoning")
        {
            ids.effort = Some(id.into());
            metadata.efforts = values
                .into_iter()
                .flatten()
                .filter_map(|value| value.get("value")?.as_str().map(str::to_owned))
                .collect();
        }
    }
    (metadata, ids)
}

pub(super) fn commands_from_update(
    message: &super::wire::AcpInbound,
    session_id: &str,
) -> Option<Vec<Value>> {
    let super::wire::AcpInbound::Notification { method, params } = message else {
        return None;
    };
    if method != "session/update"
        || params.get("sessionId").and_then(Value::as_str) != Some(session_id)
        || params
            .pointer("/update/sessionUpdate")
            .and_then(Value::as_str)
            != Some("available_commands_update")
    {
        return None;
    }
    commands_from_value(params.get("update")?)
}

pub(super) fn commands_from_value(update: &Value) -> Option<Vec<Value>> {
    Some(
        update
            .get("availableCommands")?
            .as_array()?
            .iter()
            .filter_map(|command| {
                let name = command.get("name")?.as_str()?.trim_start_matches('/');
                Some(json!({
                    "name": name,
                    "description": command.get("description").and_then(Value::as_str),
                    "source": "prompt",
                }))
            })
            .collect(),
    )
}

pub(super) fn content_text(content: &Value) -> Option<String> {
    content
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| content.as_str())
        .map(str::to_owned)
}

pub(super) fn normalize_tool_name(update: &Value, title: &str) -> String {
    match update.get("kind").and_then(Value::as_str).unwrap_or("") {
        "read" => CommonTool::Read.name().into(),
        "edit" | "delete" | "move" => CommonTool::Edit.name().into(),
        "search" => "grep".into(),
        "fetch" => "web_fetch".into(),
        _ => title.to_owned(),
    }
}

pub(super) fn merge_tool_metadata(metadata: &mut ToolMetadata, update: &Value) {
    let mut native = metadata
        .native
        .take()
        .unwrap_or_else(|| Value::Object(Default::default()));
    merge_value(&mut native, update);

    metadata.category = native
        .get("kind")
        .and_then(Value::as_str)
        .map(|kind| match kind {
            "read" => ToolCategory::Read,
            "search" => ToolCategory::Search,
            "list" => ToolCategory::List,
            "edit" | "delete" | "move" => ToolCategory::Change,
            "execute" => ToolCategory::Execute,
            "fetch" => ToolCategory::Fetch,
            "delegate" => ToolCategory::Delegate,
            _ => ToolCategory::Other,
        });
    metadata.title = native
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_owned);
    metadata.targets = native
        .get("locations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|location| {
            location
                .as_str()
                .or_else(|| location.get("path").and_then(Value::as_str))
                .or_else(|| location.get("uri").and_then(Value::as_str))
                .map(str::to_owned)
        })
        .collect();
    metadata.native = Some(native);
}

pub(super) fn tool_metadata(update: &Value) -> ToolMetadata {
    let mut metadata = ToolMetadata::default();
    merge_tool_metadata(&mut metadata, update);
    metadata
}

const PATH_KEYS: &[&str] = &["path", "file_path", "filePath"];
const OLD_TEXT_KEYS: &[&str] = &["oldText", "old_string", "oldString"];
const NEW_TEXT_KEYS: &[&str] = &["newText", "new_string", "newString"];

pub(super) fn tool_args(metadata: &ToolMetadata) -> Value {
    let native = metadata.native.as_ref();
    let mut arguments = native
        .and_then(|native| native.get("rawInput"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    promote(&mut arguments, "path", PATH_KEYS);
    promote(&mut arguments, "oldText", OLD_TEXT_KEYS);
    promote(&mut arguments, "newText", NEW_TEXT_KEYS);
    if let Some(diff) = native.and_then(first_diff_block) {
        fill_missing(&mut arguments, "path", text_field(diff, PATH_KEYS));
    }
    fill_missing(
        &mut arguments,
        "path",
        metadata.targets.first().map(String::as_str),
    );
    Value::Object(arguments)
}

fn promote(arguments: &mut serde_json::Map<String, Value>, canonical: &str, names: &[&str]) {
    let value = names
        .iter()
        .filter(|name| **name != canonical)
        .find_map(|name| arguments.remove(*name));
    if let Some(value) = value {
        arguments.entry(canonical.to_owned()).or_insert(value);
    }
}

fn fill_missing(arguments: &mut serde_json::Map<String, Value>, key: &str, value: Option<&str>) {
    if !arguments.contains_key(key)
        && let Some(value) = value
    {
        arguments.insert(key.into(), json!(value));
    }
}

fn text_field<'a>(value: &'a Value, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_str))
}

fn first_diff_block(native: &Value) -> Option<&Value> {
    content_values(native.get("content")?).find_map(|value| {
        let block = unwrap_content_block(value);
        (block.get("type").and_then(Value::as_str) == Some("diff")).then_some(block)
    })
}

fn content_values(content: &Value) -> impl Iterator<Item = &Value> {
    content
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(content))
        .iter()
}

fn unwrap_content_block(value: &Value) -> &Value {
    match value.get("type").and_then(Value::as_str) {
        Some("content") => value.get("content").unwrap_or(value),
        _ => value,
    }
}

fn merge_value(current: &mut Value, update: &Value) {
    if let (Some(current), Some(update)) = (current.as_object_mut(), update.as_object()) {
        for (key, value) in update {
            if let Some(previous) = current.get_mut(key) {
                merge_value(previous, value);
            } else {
                current.insert(key.clone(), value.clone());
            }
        }
    } else {
        *current = update.clone();
    }
}

pub(super) fn merged_tool_content(metadata: &ToolMetadata, update: &Value) -> Value {
    tool_content(metadata.native.as_ref().unwrap_or(update))
}

pub(super) fn tool_result(metadata: &ToolMetadata, update: &Value) -> Value {
    let mut result = json!({"content": merged_tool_content(metadata, update)});
    if let Some(details) = edit_result_details(metadata) {
        result["details"] = details;
    }
    result
}

fn edit_result_details(metadata: &ToolMetadata) -> Option<Value> {
    let (old, new) = edit_texts(metadata)?;
    let diff = line_diff(old, new);
    if diff.is_empty() {
        return None;
    }
    let mut details = json!({"diff": diff});
    if let Some(line) = first_changed_line(metadata) {
        details["firstChangedLine"] = json!(line);
    }
    Some(details)
}

fn edit_texts(metadata: &ToolMetadata) -> Option<(&str, &str)> {
    let native = metadata.native.as_ref()?;
    if let Some(diff) = first_diff_block(native) {
        let old = text_field(diff, OLD_TEXT_KEYS).unwrap_or("");
        let new = text_field(diff, NEW_TEXT_KEYS).unwrap_or("");
        if !old.is_empty() || !new.is_empty() {
            return Some((old, new));
        }
    }
    let raw = native.get("rawInput")?;
    let old = text_field(raw, OLD_TEXT_KEYS)?;
    let new = text_field(raw, NEW_TEXT_KEYS)?;
    Some((old, new))
}

fn line_diff(old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let mut diff = String::new();
    for edit in changed_lines(&old_lines, &new_lines) {
        match edit {
            LineEdit::Delete(line) => {
                diff.push('-');
                diff.push_str(line);
                diff.push('\n');
            }
            LineEdit::Insert(line) => {
                diff.push('+');
                diff.push_str(line);
                diff.push('\n');
            }
        }
    }
    diff
}

#[derive(Clone, Copy)]
enum LineEdit<'a> {
    Delete(&'a str),
    Insert(&'a str),
}

fn changed_lines<'a>(old: &'a [&str], new: &'a [&str]) -> Vec<LineEdit<'a>> {
    let prefix = old
        .iter()
        .zip(new.iter())
        .take_while(|(left, right)| left == right)
        .count();
    let old = &old[prefix..];
    let new = &new[prefix..];
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let old = &old[..old.len().saturating_sub(suffix)];
    let new = &new[..new.len().saturating_sub(suffix)];
    if old.len().saturating_mul(new.len()) > 1_000_000 {
        return old
            .iter()
            .copied()
            .map(LineEdit::Delete)
            .chain(new.iter().copied().map(LineEdit::Insert))
            .collect();
    }
    lcs_edits(old, new)
}

fn lcs_edits<'a>(old: &'a [&str], new: &'a [&str]) -> Vec<LineEdit<'a>> {
    let n = old.len();
    let m = new.len();
    let width = m.saturating_add(1);
    let mut dp = vec![0_u32; n.saturating_add(1).saturating_mul(width)];
    let cell = |row: usize, column: usize| row.saturating_mul(width).saturating_add(column);
    for i in 0..n {
        for j in 0..m {
            dp[cell(i + 1, j + 1)] = if old[i] == new[j] {
                dp[cell(i, j)].saturating_add(1)
            } else {
                dp[cell(i + 1, j)].max(dp[cell(i, j + 1)])
            };
        }
    }
    let mut edits = Vec::new();
    let mut i = n;
    let mut j = m;
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old[i - 1] == new[j - 1] {
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[cell(i, j - 1)] >= dp[cell(i - 1, j)]) {
            j -= 1;
            edits.push(LineEdit::Insert(new[j]));
        } else if i > 0 {
            i -= 1;
            edits.push(LineEdit::Delete(old[i]));
        }
    }
    edits.reverse();
    edits
}

fn first_changed_line(metadata: &ToolMetadata) -> Option<u64> {
    metadata
        .native
        .as_ref()?
        .get("locations")?
        .as_array()?
        .iter()
        .find_map(|location| location.get("line").and_then(Value::as_u64))
}

pub(super) fn tool_content(update: &Value) -> Value {
    let content = update
        .get("content")
        .map(normalize_content)
        .unwrap_or_else(|| json!([]));
    if content
        .as_array()
        .is_some_and(|content| !content.is_empty())
    {
        content
    } else {
        update
            .get("rawOutput")
            .map(normalize_content)
            .unwrap_or_else(|| json!([]))
    }
}

pub(super) fn normalize_content(content: &Value) -> Value {
    Value::Array(
        content_values(content)
            .filter_map(|value| match value.get("type").and_then(Value::as_str) {
                Some("content") => value.get("content").cloned(),
                Some("text" | "image" | "resource") => Some(value.clone()),
                Some("diff") => Some(json!({
                    "type": "text",
                    "text": format_diff(value),
                })),
                _ => value
                    .as_str()
                    .or_else(|| value.get("text").and_then(Value::as_str))
                    .or_else(|| value.get("output").and_then(Value::as_str))
                    .map(|text| json!({"type": "text", "text": text})),
            })
            .collect(),
    )
}

fn format_diff(value: &Value) -> String {
    let path = text_field(value, PATH_KEYS).unwrap_or("file");
    let old = text_field(value, OLD_TEXT_KEYS).unwrap_or("");
    let new = text_field(value, NEW_TEXT_KEYS).unwrap_or("");
    format!("Diff for {path}\n--- before\n{old}\n+++ after\n{new}")
}

pub(super) fn usage_update(update: &Value) -> Option<WorkerUsage> {
    let usage = update.get("usage").unwrap_or(update);
    let input = number(usage, &["inputTokens", "input"]);
    let output = number(usage, &["outputTokens", "output"]);
    let cache_read = number(usage, &["cachedInputTokens", "cacheRead"]);
    let cache_write = number(usage, &["cacheWriteInputTokens", "cacheWrite"]);
    let context_window = number(update, &["size", "contextWindow"]);
    (input + output + cache_read + cache_write + context_window > 0).then_some(WorkerUsage {
        turn: TokenUsage {
            input,
            output,
            cache_read,
            cache_write,
        },
        session: TokenUsage {
            input,
            output,
            cache_read,
            cache_write,
        },
        context_window,
    })
}

fn number(value: &Value, names: &[&str]) -> u64 {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_u64))
        .unwrap_or(0)
}

pub(super) fn find_permission_option(options: &[Value], allow: bool) -> Option<String> {
    let preferred = if allow {
        ["allow_once", "allow-once", "allow_always", "allow-always"]
    } else {
        ["reject_once", "reject-once", "deny_once", "deny-once"]
    };
    preferred.into_iter().find_map(|wanted| {
        options.iter().find_map(|option| {
            let kind = option.get("kind").and_then(Value::as_str).unwrap_or("");
            let id = option
                .get("optionId")
                .or_else(|| option.get("id"))
                .and_then(Value::as_str)?;
            (kind == wanted || id == wanted).then(|| id.to_owned())
        })
    })
}

pub(super) fn is_acceptance(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "yes" | "true" | "allow" | "accept" | "accepted" | "allow once" | "allow always"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: AcpProfile = AcpProfile {
        backend: "test-acp",
        name: "Test ACP",
        command: "test-acp",
        path_environment: "FARCASTER_TEST_ACP_PATH",
        arguments: &["acp"],
        auth_method: None,
        force_argument: Some("--force"),
    };

    #[test]
    fn session_config_options_become_neutral_catalogs() {
        let (metadata, ids) = metadata_from_session(
            &PROFILE,
            &json!({
                "configOptions": [
                    {"id":"mode","category":"mode","options":[{"value":"agent","name":"Agent"}]},
                    {"id":"model","category":"model","options":[{"value":"fast","name":"Fast"}]}
                ]
            }),
        );
        assert_eq!(metadata.models[0]["id"], "fast");
        assert_eq!(metadata.models[0]["provider"], "test-acp");
        assert_eq!(metadata.modes[0]["id"], "agent");
        assert_eq!(ids.model.as_deref(), Some("model"));
    }

    #[test]
    fn tool_content_unwraps_acp_content_blocks() {
        assert_eq!(
            tool_content(&json!({
                "content": [{
                    "type": "content",
                    "content": {"type": "text", "text": "done"}
                }]
            })),
            json!([{"type": "text", "text": "done"}])
        );
    }

    #[test]
    fn tool_metadata_merges_partial_acp_updates() {
        let mut metadata = tool_metadata(&json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "one",
            "kind": "read",
            "title": "Read file",
            "locations": [{"path": "src/main.rs", "line": 4}]
        }));
        merge_tool_metadata(
            &mut metadata,
            &json!({"sessionUpdate":"tool_call_update", "rawInput":{"path":"src/main.rs"}}),
        );
        assert_eq!(metadata.category, Some(ToolCategory::Read));
        assert_eq!(metadata.title.as_deref(), Some("Read file"));
        assert_eq!(metadata.targets, ["src/main.rs"]);
        assert_eq!(tool_args(&metadata), json!({"path":"src/main.rs"}));
        assert_eq!(metadata.native.as_ref().unwrap()["kind"], "read");
        assert_eq!(
            metadata.native.as_ref().unwrap()["rawInput"]["path"],
            "src/main.rs"
        );
    }

    #[test]
    fn completed_update_retains_content_from_an_earlier_partial_update() {
        let mut metadata = tool_metadata(&json!({
            "sessionUpdate":"tool_call_update",
            "toolCallId":"one",
            "content":[{"type":"text", "text":"earlier output"}]
        }));
        let completed = json!({
            "sessionUpdate":"tool_call_update",
            "toolCallId":"one",
            "status":"completed"
        });
        merge_tool_metadata(&mut metadata, &completed);
        assert_eq!(
            merged_tool_content(&metadata, &completed),
            json!([{"type":"text", "text":"earlier output"}])
        );
    }

    #[test]
    fn edit_payloads_use_canonical_args_and_diff_details() {
        assert_eq!(
            tool_args(&tool_metadata(&json!({
                "kind": "edit",
                "rawInput": {
                    "filePath": "src/main.rs",
                    "old_string": "old",
                    "new_string": "new\nline"
                }
            }))),
            json!({
                "path": "src/main.rs",
                "oldText": "old",
                "newText": "new\nline"
            })
        );

        let metadata = tool_metadata(&json!({
            "kind": "edit",
            "locations": [{"path": "src/lib.rs", "line": 10}],
            "rawInput": {"path": "src/lib.rs"},
            "content": [{
                "type": "diff",
                "path": "src/lib.rs",
                "oldText": "fn a() {}\nfn keep() {}",
                "newText": "fn a() {}\nfn keep() {}\nfn b() {}"
            }]
        }));
        assert_eq!(tool_args(&metadata), json!({"path": "src/lib.rs"}));
        assert_eq!(
            tool_result(&metadata, &json!({}))["details"],
            json!({
                "diff": "+fn b() {}\n",
                "firstChangedLine": 10
            })
        );
    }

    #[test]
    fn full_file_acp_diffs_count_only_changed_lines() {
        let old = (0..80)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut new_lines: Vec<String> = (0..80).map(|n| format!("line {n}")).collect();
        new_lines[10] = "changed".into();
        new_lines.insert(40, "inserted".into());
        let metadata = tool_metadata(&json!({
            "kind": "edit",
            "content": [{
                "type": "diff",
                "path": "big.rs",
                "oldText": old,
                "newText": new_lines.join("\n")
            }]
        }));
        assert_eq!(
            tool_result(&metadata, &json!({}))["details"]["diff"],
            json!("-line 10\n+changed\n+inserted\n")
        );
    }

    #[test]
    fn execute_kind_does_not_guess_bash() {
        assert_eq!(
            normalize_tool_name(&json!({"kind":"execute"}), "Run database migration"),
            "Run database migration"
        );
    }

    #[test]
    fn available_commands_update_becomes_prompt_commands() {
        let message = super::super::wire::AcpInbound::Notification {
            method: "session/update".into(),
            params: json!({
                "sessionId": "one",
                "update": {
                    "sessionUpdate": "available_commands_update",
                    "availableCommands": [{"name":"/review","description":"Review changes"}]
                }
            }),
        };
        assert_eq!(
            commands_from_update(&message, "one"),
            Some(vec![json!({
                "name": "review",
                "description": "Review changes",
                "source": "prompt"
            })])
        );
        assert_eq!(commands_from_update(&message, "other"), None);
    }
}
