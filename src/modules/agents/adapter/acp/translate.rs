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
        // ACP's `execute` kind is broader than a shell command. Keep the
        // agent-provided title rather than manufacturing bash semantics.
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

pub(super) fn tool_args(metadata: &ToolMetadata) -> Value {
    metadata
        .native
        .as_ref()
        .and_then(|native| native.get("rawInput"))
        .cloned()
        .unwrap_or_else(|| json!({}))
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
    let values = content
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(content));
    Value::Array(
        values
            .iter()
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
    let path = value.get("path").and_then(Value::as_str).unwrap_or("file");
    let old = value.get("oldText").and_then(Value::as_str).unwrap_or("");
    let new = value.get("newText").and_then(Value::as_str).unwrap_or("");
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
