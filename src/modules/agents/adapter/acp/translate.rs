use serde_json::{Value, json};

use super::AcpProfile;
use crate::agents::{CommonTool, TokenUsage, WorkerUsage};

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
        "execute" => CommonTool::Bash.name().into(),
        "search" => "grep".into(),
        "fetch" => "web_fetch".into(),
        _ => title.to_owned(),
    }
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
}
