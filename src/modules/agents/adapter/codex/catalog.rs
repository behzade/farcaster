use std::{
    io::BufReader,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Value, json};

use super::{connection::CodexConnection, contract::CodexClientInfo};
use crate::agents::{CommonTool, DiscoveredHistory, DiscoveredSession, DiscoveredUsage};

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

pub(in crate::modules::agents::adapter) fn discover(
    locator_root: &Path,
    query: &str,
) -> Result<Vec<DiscoveredSession>, String> {
    with_connection(|connection| {
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
                    if let Some(summary) = summary(locator_root, thread, archived)? {
                        sessions.push(summary);
                    }
                }
            }
        }
        Ok(sessions)
    })
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
    let project = PathBuf::from(cwd);
    if !project.is_dir() {
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
        Some(kind @ ("commandExecution" | "mcpToolCall" | "fileChange" | "webSearch")) => {
            history_tool_messages(item, kind)
        }
        _ => Vec::new(),
    }
}

fn history_tool_messages(item: &Value, kind: &str) -> Vec<Value> {
    let Some(id) = item.get("id").and_then(Value::as_str) else {
        return Vec::new();
    };
    let (name, arguments) = history_tool_call(item, kind);
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
                "name": name,
                "arguments": arguments,
            }],
        }),
        json!({
            "role": "toolResult",
            "toolCallId": id,
            "toolName": name,
            "content": history_tool_output(item, kind, is_error),
            "isError": is_error,
        }),
    ]
}

fn history_tool_call(item: &Value, kind: &str) -> (String, Value) {
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
            let read = item
                .get("commandActions")
                .and_then(Value::as_array)
                .filter(|actions| actions.len() == 1)
                .and_then(|actions| actions.first())
                .filter(|action| action.get("type").and_then(Value::as_str) == Some("read"));
            if let Some(action) = read {
                (
                    CommonTool::Read.name().into(),
                    json!({"path": action.get("path").cloned().unwrap_or(Value::Null)}),
                )
            } else {
                (
                    CommonTool::Bash.name().into(),
                    json!({"command": item.get("command").cloned().unwrap_or(Value::Null)}),
                )
            }
        }
        "webSearch" => (
            "web_search".into(),
            json!({"query": history_web_search_query(item)}),
        ),
        _ => {
            let name = item
                .get("tool")
                .and_then(Value::as_str)
                .or_else(|| item.get("name").and_then(Value::as_str))
                .or_else(|| item.get("server").and_then(Value::as_str))
                .unwrap_or(kind);
            let arguments = item.get("arguments").cloned().unwrap_or_else(|| json!({}));
            (name.to_owned(), arguments)
        }
    }
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
                .then(|| history_web_search_query(item))
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

fn history_web_search_query(item: &Value) -> Option<&str> {
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
mod tests {
    use super::*;

    #[test]
    fn translates_thread_metadata() -> Result<(), String> {
        let project = std::env::current_dir().map_err(|error| error.to_string())?;
        let value = json!({
            "id": "thread-1",
            "cwd": project,
            "name": "Fix tests",
            "preview": "Please fix tests",
            "updatedAt": SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_secs(),
            "status": {"type": "active"},
            "tokenUsage": {"total": {"inputTokens": 100, "outputTokens": 20, "cachedInputTokens": 80}},
        });
        let session = summary(project.as_path(), &value, false)?.ok_or("summary")?;
        assert_eq!(session.harness, "codex-cli");
        assert!(session.is_running);
        assert_eq!(session.title, "Fix tests");
        assert_eq!(session.usage.input, 20);
        assert_eq!(session.usage.output, 20);
        assert_eq!(session.usage.cache_read, 80);
        assert_eq!(session.usage.total, 120);
        Ok(())
    }

    #[test]
    fn discovery_requests_native_agent_sources_explicitly() {
        let params = thread_list_params(false, "", AGENT_SOURCE_KINDS);
        assert_eq!(
            params["sourceKinds"],
            json!([
                "subAgent",
                "subAgentReview",
                "subAgentCompact",
                "subAgentThreadSpawn",
                "subAgentOther"
            ])
        );
        assert!(params["searchTerm"].is_null());
    }

    #[test]
    fn translates_thread_messages() {
        assert_eq!(
            history_messages(&json!({
                "type": "userMessage",
                "content": [{"type": "text", "text": "hello"}],
            }))[0]["role"],
            "user"
        );
        assert_eq!(
            history_messages(&json!({"type": "agentMessage", "text": "done"}))[0]["role"],
            "assistant"
        );
    }

    #[test]
    fn translates_historical_command_calls_and_output() {
        let messages = history_messages(&json!({
            "type": "commandExecution",
            "id": "command-1",
            "command": "cargo test",
            "commandActions": [],
            "status": "completed",
            "aggregatedOutput": "ok",
        }));

        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].pointer("/content/0/type"),
            Some(&json!("toolCall"))
        );
        assert_eq!(messages[0].pointer("/content/0/name"), Some(&json!("bash")));
        assert_eq!(
            messages[0].pointer("/content/0/arguments/command"),
            Some(&json!("cargo test"))
        );
        assert_eq!(messages[1]["role"], "toolResult");
        assert_eq!(messages[1].pointer("/content/0/text"), Some(&json!("ok")));
        assert_eq!(messages[1]["isError"], false);
    }

    #[test]
    fn translates_historical_mcp_failures() {
        let messages = history_messages(&json!({
            "type": "mcpToolCall",
            "id": "mcp-1",
            "server": "github",
            "tool": "get_pull_request",
            "arguments": {"number": 42},
            "status": "failed",
            "result": null,
            "error": {"message": "not found"},
        }));

        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].pointer("/content/0/name"),
            Some(&json!("get_pull_request"))
        );
        assert_eq!(
            messages[0].pointer("/content/0/arguments/number"),
            Some(&json!(42))
        );
        assert_eq!(
            messages[1].pointer("/content/0/text"),
            Some(&json!("not found"))
        );
        assert_eq!(messages[1]["isError"], true);
    }

    #[test]
    fn translates_historical_file_changes_and_web_searches() {
        let file = history_messages(&json!({
            "type": "fileChange",
            "id": "change-1",
            "changes": [{"path": "src/main.rs", "diff": "+fn main() {}"}],
            "status": "completed",
        }));
        assert_eq!(file[0].pointer("/content/0/name"), Some(&json!("edit")));
        assert_eq!(
            file[0].pointer("/content/0/arguments/path"),
            Some(&json!("src/main.rs"))
        );
        assert_eq!(
            file[1].pointer("/content/0/text"),
            Some(&json!("Applied patch"))
        );

        let search = history_messages(&json!({
            "type": "webSearch",
            "id": "search-1",
            "query": "Codex app-server",
        }));
        assert_eq!(
            search[0].pointer("/content/0/name"),
            Some(&json!("web_search"))
        );
        assert_eq!(
            search[0].pointer("/content/0/arguments/query"),
            Some(&json!("Codex app-server"))
        );
    }
}
