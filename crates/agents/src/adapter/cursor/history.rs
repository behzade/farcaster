use super::bridge::Bridge;
use crate::{Backend, DiscoveredHistory, DiscoveredSession};
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path, time::UNIX_EPOCH};

pub(super) fn load(bridge: &Bridge, id: &str, project: &Path) -> Result<DiscoveredHistory, String> {
    let mut messages = Vec::new();
    let mut seen = HashSet::new();
    for page in 0..1000 {
        let response = bridge.agent("ListAgentMessages",json!({"agentId":id,"options":{"runtime":"RUNTIME_LOCAL","cwd":project,"limit":200,"offset":page*200}}))?;
        let items = response["messages"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        for message in items {
            let Some(id) = message["uuid"].as_str() else {
                return Err("Cursor SDK message has no identity".into());
            };
            if !seen.insert(id.to_owned()) {
                return Err("Cursor SDK repeated a history page".into());
            }
            messages.extend(expand_message(message, id)?);
        }
        if items.len() < 200 {
            return Ok(DiscoveredHistory {
                messages: std::sync::Arc::new(messages),
                model: None,
                thinking_level: None,
                prompt_deliveries: None,
            });
        }
    }
    Err("Cursor SDK history exceeded 1000 pages".into())
}

fn expand_message(message: &Value, id: &str) -> Result<Vec<Value>, String> {
    let payload = &message["message"];
    // Local SDK history stores a whole conversation turn in each entry, even
    // though the entry's outer type is "user".
    if let Some(("agentConversationTurn", turn)) =
        variant(payload, "turn", &["agentConversationTurn"])
    {
        let mut messages = Vec::new();
        if let Some(text) = turn.pointer("/userMessage/text").and_then(Value::as_str) {
            messages.push(json!({"id":format!("{id}:user"),"role":"user","content":[{"type":"text","text":text}]}));
        }
        let mut content = Vec::new();
        for (index, step) in turn["steps"].as_array().into_iter().flatten().enumerate() {
            let Some((kind, value)) = variant(
                step,
                "message",
                &["assistantMessage", "thinkingMessage", "toolCall"],
            ) else {
                continue;
            };
            match kind {
                "assistantMessage" => {
                    if let Some(text) = value.get("text").and_then(Value::as_str) {
                        content.push(json!({"type":"text","text":text}));
                    }
                }
                "thinkingMessage" => {
                    if let Some(text) = value.get("text").and_then(Value::as_str) {
                        content.push(json!({"type":"thinking","thinking":text}));
                    }
                }
                "toolCall" => {
                    let native = value;
                    let (kind, value) =
                        if let Some(kind) = native.pointer("/tool/case").and_then(Value::as_str) {
                            (kind, &native["tool"]["value"])
                        } else {
                            native
                                .as_object()
                                .and_then(|fields| {
                                    fields.iter().find(|(key, _)| key.ends_with("ToolCall"))
                                })
                                .map(|(key, value)| (key.as_str(), value))
                                .ok_or("Cursor history tool has no type")?
                        };
                    let name = kind.strip_suffix("ToolCall").unwrap_or(kind);
                    let call_id = native["toolCallId"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("{id}:tool:{index}"));
                    content.push(json!({"type":"toolCall","id":call_id,"name":name,
                        "arguments":value["args"],"toolMetadata":{"native":native}}));
                    flush_assistant(&mut messages, &mut content, id);
                    if let Some(result) = value.get("result") {
                        // Stored protobuf results use a oneof; live SDK deltas use status/value.
                        let result = stored_result(result);
                        messages.push(
                            json!({"id":format!("{id}:result:{index}"),"role":"toolResult",
                            "toolCallId":call_id,"toolName":name,
                            "content":super::events::tool_result(&result),
                            "isError":super::events::tool_failed(&result)}),
                        );
                    }
                }
                _ => {}
            }
        }
        flush_assistant(&mut messages, &mut content, id);
        return Ok(messages);
    }
    if let Some(("shellConversationTurn", turn)) =
        variant(payload, "turn", &["shellConversationTurn"])
    {
        let call_id = format!("{id}:shell");
        let mut messages = vec![
            json!({"id":format!("{id}:assistant"),"role":"assistant","content":[{
                "type":"toolCall","id":call_id,"name":"shell","arguments":turn["shellCommand"]
            }]}),
        ];
        if let Some(output) = turn.get("shellOutput") {
            messages.push(
                json!({"id":format!("{id}:result"),"role":"toolResult","toolCallId":call_id,
                "toolName":"shell","content":super::events::tool_result(output),
                "isError":output["exitCode"].as_i64().is_some_and(|code| code != 0)}),
            );
        }
        return Ok(messages);
    }
    let role = message["type"].as_str().unwrap_or_default();
    if !matches!(role, "user" | "assistant") {
        return Ok(Vec::new());
    }
    let mut body = payload.get("message").unwrap_or(payload).clone();
    if !body.is_object() {
        return Err("Cursor SDK history message is not an object".into());
    }
    body["role"] = role.into();
    body["id"] = id.into();
    Ok(vec![body])
}

// The native bridge exposes protobuf oneofs as case/value; the JS SDK's
// toJSON emits canonical protobuf JSON with the case as a property name.
fn variant<'a>(
    value: &'a Value,
    field: &str,
    cases: &[&'static str],
) -> Option<(&'static str, &'a Value)> {
    for &case in cases {
        if value[field]["case"] == case {
            return Some((case, &value[field]["value"]));
        }
        if let Some(body) = value.get(case) {
            return Some((case, body));
        }
    }
    None
}

fn flush_assistant(messages: &mut Vec<Value>, content: &mut Vec<Value>, id: &str) {
    if !content.is_empty() {
        let suffix = if messages
            .iter()
            .any(|message| message["role"] == "assistant")
        {
            format!("assistant:{}", messages.len())
        } else {
            "assistant".into()
        };
        messages.push(json!({"id":format!("{id}:{suffix}"),"role":"assistant","content":std::mem::take(content)}));
    }
}

fn stored_result(result: &Value) -> Value {
    let oneof = result.get("result").unwrap_or(result);
    match oneof["case"].as_str() {
        Some("success") => json!({"status":"success","value":oneof["value"]}),
        Some(_) => json!({"status":"error","error":oneof["value"]}),
        None => {
            if let Some(value) = result.get("success") {
                json!({"status":"success","value":value})
            } else if result.get("status").is_some() {
                result.clone()
            } else {
                json!({"status":"error","error":result})
            }
        }
    }
}

pub(super) fn discover(
    bridge: &Bridge,
    root: &Path,
    query: &str,
) -> Result<Vec<DiscoveredSession>, String> {
    let mut result = Vec::new();
    let mut cursor = String::new();
    let mut seen = HashSet::new();
    let query = query.to_lowercase();
    for _ in 0..1000 {
        let response = bridge.agent(
            "ListAgents",
            json!({"options":{"runtime":"RUNTIME_LOCAL","limit":100,"cursor":cursor}}),
        )?;
        for agent in response["items"].as_array().into_iter().flatten() {
            let Some(id) = agent["agentId"].as_str() else {
                continue;
            };
            let Some(cwd) = agent.pointer("/local/cwd").and_then(Value::as_str) else {
                continue;
            };
            let project = std::path::PathBuf::from(cwd);
            if !project.is_absolute() || farcaster_projects::is_temporary_project(&project) {
                continue;
            }
            let title = agent["name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("Cursor session")
                .to_owned();
            let search = format!("{title} {cwd} Cursor");
            if !search.to_lowercase().contains(&query) {
                continue;
            }
            let timestamp = agent["lastModified"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let modified = time::OffsetDateTime::parse(
                &timestamp,
                &time::format_description::well_known::Rfc3339,
            )
            .ok()
            .map(std::time::SystemTime::from)
            .unwrap_or(UNIX_EPOCH);
            result.push(DiscoveredSession {
                id: id.into(),
                harness: Backend::Cursor,
                path: crate::adapter::main_session::external_session_path(
                    root,
                    Backend::Cursor,
                    id,
                ),
                project,
                title,
                first_user_message: String::new(),
                timestamp,
                parent_session: None,
                modified,
                message_count: 0,
                usage: Default::default(),
                archived: agent["archived"].as_bool().unwrap_or(false),
                is_running: agent["status"] == "AGENT_INFO_STATUS_RUNNING",
                model: None,
                thinking_level: None,
                search,
            });
        }
        cursor = response["nextCursor"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if cursor.is_empty() {
            return Ok(result);
        }
        if !seen.insert(cursor.clone()) {
            return Err("Cursor SDK repeated a session cursor".into());
        }
    }
    Err("Cursor SDK session list exceeded 1000 pages".into())
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
