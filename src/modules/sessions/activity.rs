use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::sessions::UsageSummary;

const MAX_ACTIVITY_CHARS: usize = 160;
const MAX_TOOL_TARGET_CHARS: usize = 120;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentOutcome {
    Complete,
    Failed,
    Incomplete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentLifecycle {
    #[allow(dead_code)]
    NeedsInput,
    Working,
    Unknown,
    Completed(AgentOutcome),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentToolActivity {
    pub name: String,
    pub target: String,
    pub failed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentActivity {
    pub session_id: String,
    pub session_path: PathBuf,
    pub role: String,
    pub activity: String,
    pub lifecycle: AgentLifecycle,
    pub current_tool: Option<AgentToolActivity>,
    pub recent_tool: Option<AgentToolActivity>,
    pub tool_call_count: usize,
    pub limited: bool,
    pub usage: UsageSummary,
    pub started: SystemTime,
    pub ended: Option<SystemTime>,
    pub elapsed: Option<Duration>,
}

#[derive(Default)]
pub(crate) struct ActivityBuilder {
    tools: HashMap<String, AgentToolActivity>,
    outstanding_tool_ids: Vec<String>,
    recent_tool: Option<AgentToolActivity>,
    tool_call_count: usize,
    outcome: Option<AgentOutcome>,
    terminal_time: Option<SystemTime>,
}

impl ActivityBuilder {
    pub(crate) fn observe_entry(&mut self, entry: &Value) {
        let Some(message) = entry
            .get("message")
            .filter(|_| entry.get("type").and_then(Value::as_str) == Some("message"))
        else {
            return;
        };
        let observed_at = entry_timestamp(entry, message);
        match message.get("role").and_then(Value::as_str) {
            Some("assistant") => self.observe_assistant(message, observed_at),
            Some("toolResult") => self.observe_tool_result(message),
            Some("user") => {
                self.outcome = None;
                self.terminal_time = None;
            }
            _ => {}
        }
    }

    fn observe_assistant(&mut self, message: &Value, observed_at: Option<SystemTime>) {
        if let Some(blocks) = message.get("content").and_then(Value::as_array) {
            for block in blocks {
                if block.get("type").and_then(Value::as_str) != Some("toolCall") {
                    continue;
                }
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Tool")
                    .to_owned();
                let arguments = block.get("arguments").unwrap_or(&Value::Null);
                let target = tool_target(arguments);
                let tool = AgentToolActivity {
                    name,
                    target,
                    failed: false,
                };
                self.tool_call_count = self.tool_call_count.saturating_add(1);
                if !id.is_empty() {
                    self.tools.insert(id.clone(), tool);
                    self.outstanding_tool_ids.push(id);
                } else {
                    self.recent_tool = Some(tool);
                }
            }
        }
        self.outcome = match message.get("stopReason").and_then(Value::as_str) {
            Some("stop") => Some(AgentOutcome::Complete),
            Some("error") => Some(AgentOutcome::Failed),
            Some("aborted" | "length") => Some(AgentOutcome::Incomplete),
            Some("toolUse") | None => None,
            Some(_) => None,
        };
        self.terminal_time = self.outcome.and(observed_at);
    }

    fn observe_tool_result(&mut self, message: &Value) {
        let id = message
            .get("toolCallId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let failed = message
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut tool = self.tools.remove(id).unwrap_or_else(|| AgentToolActivity {
            name: message
                .get("toolName")
                .and_then(Value::as_str)
                .unwrap_or("Tool")
                .to_owned(),
            target: String::new(),
            failed,
        });
        tool.failed = failed;
        self.recent_tool = Some(tool);
        self.outstanding_tool_ids
            .retain(|outstanding| outstanding != id);
        self.outcome = None;
        self.terminal_time = None;
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn finish(
        self,
        session_id: String,
        session_path: PathBuf,
        title: &str,
        first_user_message: &str,
        usage: UsageSummary,
        started: SystemTime,
        modified: SystemTime,
        is_running: bool,
        limited: bool,
    ) -> AgentActivity {
        let lifecycle = if is_running {
            AgentLifecycle::Working
        } else if let Some(outcome) = self.outcome {
            AgentLifecycle::Completed(outcome)
        } else {
            AgentLifecycle::Unknown
        };
        let ended = matches!(lifecycle, AgentLifecycle::Completed(_))
            .then_some(self.terminal_time.unwrap_or(modified));
        let elapsed = ended.and_then(|ended| ended.duration_since(started).ok());
        let unmatched_tool = self
            .outstanding_tool_ids
            .iter()
            .rev()
            .find_map(|id| self.tools.get(id))
            .cloned();
        let (current_tool, recent_tool) = if matches!(lifecycle, AgentLifecycle::Completed(_)) {
            (None, unmatched_tool.or(self.recent_tool))
        } else {
            (unmatched_tool, self.recent_tool)
        };
        AgentActivity {
            session_id,
            session_path,
            role: role_label(title),
            activity: bounded(first_user_message, MAX_ACTIVITY_CHARS),
            lifecycle,
            current_tool,
            recent_tool,
            tool_call_count: self.tool_call_count,
            limited,
            usage,
            started,
            ended,
            elapsed,
        }
    }
}

fn entry_timestamp(entry: &Value, message: &Value) -> Option<SystemTime> {
    entry
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_iso_timestamp)
        .or_else(|| {
            message
                .get("timestamp")
                .and_then(Value::as_u64)
                .map(|millis| SystemTime::UNIX_EPOCH + Duration::from_millis(millis))
        })
}

pub(crate) fn parse_iso_timestamp(value: &str) -> Option<SystemTime> {
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()
        .map(SystemTime::from)
}

fn role_label(title: &str) -> String {
    let role = bounded(title, 28);
    if role.is_empty() {
        "Agent".into()
    } else {
        role
    }
}

fn tool_target(arguments: &Value) -> String {
    for key in ["path", "command", "script", "query", "pattern", "action"] {
        if let Some(value) = arguments.get(key).and_then(Value::as_str)
            && !value.is_empty()
        {
            return bounded(value, MAX_TOOL_TARGET_CHARS);
        }
    }
    String::new()
}

fn bounded(value: &str, max: usize) -> String {
    let mut result = value.chars().take(max).collect::<String>();
    if value.chars().count() > max {
        result.push('…');
    }
    result
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
