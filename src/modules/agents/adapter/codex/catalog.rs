use std::{
    io::BufReader,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Value, json};

use super::{connection::CodexConnection, contract::CodexClientInfo, tool};
use crate::agents::{DiscoveredHistory, DiscoveredSession, DiscoveredUsage};

use super::super::{
    child_stderr,
    main_session::{external_session_locator, external_session_path},
};

const INTERACTIVE_SOURCE_KINDS: &[&str] = &["cli", "vscode", "exec", "appServer", "unknown"];
const AGENT_SOURCE_KINDS: &[&str] = &[
    "subAgent",
    "subAgentReview",
    "subAgentCompact",
    "subAgentThreadSpawn",
    "subAgentOther",
];
const EPHEMERAL_MODELS: &[&str] = &["codex-auto-review"];

pub(in crate::modules::agents::adapter) fn discover(
    locator_root: &Path,
    query: &str,
) -> Result<Vec<DiscoveredSession>, String> {
    with_connection_and_home(|connection, home| {
        discover_with_client(connection, home, locator_root, query)
    })
}

pub(super) fn discover_with_client(
    connection: &mut CatalogConnection,
    home: &Path,
    locator_root: &Path,
    query: &str,
) -> Result<Vec<DiscoveredSession>, String> {
    let mut sessions = Vec::new();
    for archived in [false, true] {
        for source_kinds in [INTERACTIVE_SOURCE_KINDS, AGENT_SOURCE_KINDS] {
            let id = connection.send_request(
                "thread/list",
                thread_list_params(archived, query, source_kinds),
            )?;
            let response: Value = connection.wait_response(&id)?;
            for thread in response
                .get("data")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let mut thread = thread.clone();
                if let Some(id) = string(&thread, &["id"]) {
                    if let Some(project) = super::transfer::saved_project(
                        &super::transfer::project_database(home),
                        id,
                    )? {
                        thread["cwd"] = json!(project);
                    }
                }
                if let Some(summary) = summary(locator_root, &thread, archived)? {
                    sessions.push(summary);
                }
            }
        }
    }
    Ok(sessions)
}

fn thread_list_params(archived: bool, query: &str, source_kinds: &[&str]) -> Value {
    json!({
        "archived": archived,
        "limit": 100,
        "searchTerm": (!query.is_empty()).then_some(query),
        "sortKey": "updated_at",
        "sortDirection": "desc",
        "sourceKinds": source_kinds,
    })
}

pub(in crate::modules::agents::adapter) fn rename_session(
    session_id: &str,
    name: &str,
) -> Result<(), String> {
    with_connection(|connection| {
        let id = connection.send_request(
            "thread/name/set",
            json!({"threadId": session_id, "name": name}),
        )?;
        connection.wait_response::<Value>(&id).map(|_| ())
    })
}

pub(in crate::modules::agents::adapter) fn delete_session(session_id: &str) -> Result<(), String> {
    with_connection(|connection| {
        let id = connection.send_request("thread/delete", json!({"threadId": session_id}))?;
        connection.wait_response::<Value>(&id).map(|_| ())
    })
}

pub(in crate::modules::agents::adapter) fn load_history(
    path: &Path,
) -> Result<DiscoveredHistory, String> {
    let locator = external_session_locator("codex-cli", path)
        .ok_or_else(|| format!("invalid Codex session locator: {}", path.display()))?;
    with_connection_and_home(|connection, codex_home| {
        let id = connection.send_request(
            "thread/read",
            json!({"threadId": locator, "includeTurns": true}),
        )?;
        let response: Value = connection.wait_response(&id)?;
        let thread = response.get("thread").unwrap_or(&response);
        let mut messages = Vec::new();
        for turn in thread
            .get("turns")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            for item in turn
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                messages.extend(history_messages(item));
            }
        }
        let identity = stored_identity(codex_home, &locator)?;
        let (model, thinking_level) = identity.map_or((None, None), |identity| {
            (Some((identity.provider, identity.model)), identity.effort)
        });
        Ok(DiscoveredHistory {
            messages,
            model,
            thinking_level,
        })
    })
}

type CatalogConnection = CodexConnection<BufReader<ChildStdout>, ChildStdin>;

fn with_connection<T>(
    operation: impl FnOnce(&mut CatalogConnection) -> Result<T, String>,
) -> Result<T, String> {
    with_connection_and_home(|connection, _| operation(connection))
}

fn with_connection_and_home<T>(
    operation: impl FnOnce(&mut CatalogConnection, &Path) -> Result<T, String>,
) -> Result<T, String> {
    let program = std::env::var_os("FARCASTER_CODEX_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| "codex".into());
    let mut command = Command::new(program);
    command.args(["app-server", "--stdio"]);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start Codex catalog app-server: {error}"))?;
    child_stderr::capture(&mut child, "codex-catalog")?;
    let result = connect(&mut child)
        .and_then(|(mut connection, codex_home)| operation(&mut connection, &codex_home));
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn connect(child: &mut Child) -> Result<(CatalogConnection, PathBuf), String> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Codex catalog stdin must be piped".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Codex catalog stdout must be piped".to_owned())?;
    let mut connection = CodexConnection::new(BufReader::new(stdout), stdin);
    let initialized = connection.initialize(CodexClientInfo {
        name: "farcaster-catalog".into(),
        title: Some("Farcaster".into()),
        version: env!("CARGO_PKG_VERSION").into(),
    })?;
    Ok((connection, PathBuf::from(initialized.codex_home)))
}

struct CodexIdentity {
    provider: String,
    model: String,
    effort: Option<String>,
}

fn stored_identity(codex_home: &Path, thread_id: &str) -> Result<Option<CodexIdentity>, String> {
    let database = codex_home.join("state_5.sqlite");
    if !database.is_file() {
        return Ok(None);
    }
    let connection = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("open Codex state database {}: {error}", database.display()))?;
    connection
        .query_row(
            "SELECT model_provider, model, reasoning_effort FROM threads WHERE id = ?1",
            params![thread_id],
            |row| {
                let provider = row.get(0)?;
                let model = row.get::<_, Option<String>>(1)?;
                let effort = row.get(2)?;
                Ok(model.map(|model| CodexIdentity {
                    provider,
                    model,
                    effort,
                }))
            },
        )
        .optional()
        .map(|identity| identity.flatten())
        .map_err(|error| format!("read Codex session identity for {thread_id}: {error}"))
}

fn summary(
    locator_root: &Path,
    thread: &Value,
    archived: bool,
) -> Result<Option<DiscoveredSession>, String> {
    let Some(id) = string(thread, &["id"]) else {
        return Ok(None);
    };
    let Some(cwd) = string(thread, &["cwd"]) else {
        return Ok(None);
    };
    // Approval reviews use the guardian source; catalog model metadata may be absent.
    if thread
        .pointer("/source/subAgent/other")
        .and_then(Value::as_str)
        == Some("guardian")
        || string(thread, &["model"]).is_some_and(|model| EPHEMERAL_MODELS.contains(&model))
    {
        return Ok(None);
    }
    let project = PathBuf::from(cwd);
    if !project.is_dir() || crate::projects::is_temporary_project(&project) {
        return Ok(None);
    }
    let title = string(thread, &["name", "title", "preview"])
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("New Codex session")
        .to_owned();
    let first_user_message = string(thread, &["preview"]).unwrap_or_default().to_owned();
    let modified = timestamp(
        thread,
        &["updatedAt", "updated_at", "createdAt", "created_at"],
    );
    let timestamp = string(thread, &["createdAt", "created_at"])
        .unwrap_or_default()
        .to_owned();
    let parent_session = string(thread, &["parentThreadId", "parent_thread_id"])
        .map(str::to_owned)
        .or_else(|| {
            crate::modules::agents::core::CallerRegistry::shared().session_parent("codex-cli", id)
        });
    let is_running = status(thread).is_some_and(|status| {
        matches!(status, "active" | "running" | "inProgress" | "in_progress")
    });
    let path = external_session_path(locator_root, "codex-cli", id);
    let search = format!("{title} {first_user_message} {cwd} codex");
    Ok(Some(DiscoveredSession {
        id: id.to_owned(),
        harness: "codex-cli".into(),
        path,
        project,
        title,
        first_user_message,
        timestamp,
        parent_session,
        modified,
        message_count: thread
            .get("turns")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        usage: codex_usage(thread),
        archived,
        is_running,
        search,
    }))
}

fn codex_usage(thread: &Value) -> DiscoveredUsage {
    let usage = thread
        .pointer("/tokenUsage/total")
        .or_else(|| thread.pointer("/usage/total"));
    let Some(usage) = usage else {
        return DiscoveredUsage::default();
    };
    let reported_input = usage
        .get("inputTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .get("outputTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_read = usage
        .get("cachedInputTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_write = usage
        .get("cacheWriteInputTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let input = reported_input.saturating_sub(cache_read.saturating_add(cache_write));
    DiscoveredUsage {
        input,
        output,
        cache_read,
        cache_write,
        total: input
            .saturating_add(output)
            .saturating_add(cache_read)
            .saturating_add(cache_write),
        cost_micros: 0,
    }
}

fn history_messages(item: &Value) -> Vec<Value> {
    match item.get("type").and_then(Value::as_str) {
        Some("userMessage") => vec![json!({
            "role": "user",
            "content": text_content(item.get("content")),
        })],
        Some("agentMessage") => vec![json!({
            "role": "assistant",
            "content": [{"type": "text", "text": string(item, &["text"]).unwrap_or_default()}],
        })],
        Some("reasoning") => vec![json!({
            "role": "assistant",
            "content": [{"type": "thinking", "thinking": reasoning_text(item)}],
        })],
        Some(kind) if tool::is_tool_kind(kind) => history_tool_messages(item, kind),
        _ => Vec::new(),
    }
}

fn history_tool_messages(item: &Value, kind: &str) -> Vec<Value> {
    let Some(id) = item.get("id").and_then(Value::as_str) else {
        return Vec::new();
    };
    let projection = tool::project(item, kind);
    let is_error = item
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| matches!(status, "failed" | "declined"));
    vec![
        json!({
            "role": "assistant",
            "content": [{
                "type": "toolCall",
                "id": id,
                "name": projection.name,
                "arguments": projection.args,
                "toolMetadata": projection.metadata,
            }],
        }),
        json!({
            "role": "toolResult",
            "toolCallId": id,
            "toolName": projection.name,
            "content": history_tool_output(item, kind, is_error),
            "isError": is_error,
        }),
    ]
}

fn history_tool_output(item: &Value, kind: &str, is_error: bool) -> Vec<Value> {
    if kind == "mcpToolCall" {
        if is_error {
            return item
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(|text| vec![json!({"type": "text", "text": text})])
                .unwrap_or_default();
        }
        return item
            .pointer("/result/content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
    }
    let output = item
        .get("aggregatedOutput")
        .and_then(Value::as_str)
        .or_else(|| {
            (kind == "webSearch")
                .then(|| tool::web_search_query(item))
                .flatten()
        })
        .unwrap_or_else(|| {
            if kind == "fileChange" && !is_error {
                "Applied patch"
            } else {
                ""
            }
        });
    vec![json!({"type": "text", "text": output})]
}

fn text_content(content: Option<&Value>) -> Vec<Value> {
    content
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| {
            let text = string(part, &["text"])?;
            Some(json!({"type": "text", "text": text}))
        })
        .collect()
}

fn reasoning_text(item: &Value) -> String {
    string(item, &["text"])
        .map(str::to_owned)
        .or_else(|| {
            item.get("summary").and_then(Value::as_array).map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| string(part, &["text"]))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        })
        .unwrap_or_default()
}

fn status(value: &Value) -> Option<&str> {
    value
        .get("status")
        .and_then(|status| status.as_str().or_else(|| status.get("type")?.as_str()))
}

fn string<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| value.get(*key)?.as_str())
}

fn timestamp(value: &Value, keys: &[&str]) -> SystemTime {
    let raw = keys.iter().find_map(|key| value.get(*key));
    let seconds = raw
        .and_then(|value| value.as_u64().or_else(|| value.as_i64()?.try_into().ok()))
        .unwrap_or(0);
    if seconds == 0 {
        SystemTime::now()
    } else {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
