use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use farcaster_contracts::WorkerStatus;

use crate::{SessionSummary, UsageSummary};

const MAX_ACTIVITY_CHARS: usize = 160;
const MAX_TOOL_TARGET_CHARS: usize = 120;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WorkerActivityKey {
    pub path: PathBuf,
    pub native: crate::NativeSessionIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionStatusRecord {
    App(crate::AppSessionId),
    Native(WorkerActivityKey),
}

impl SessionStatusRecord {
    pub fn for_row(row: &SessionSummary) -> Self {
        crate::AppSessionId::new(row.app_session_id)
            .map_or_else(|| Self::Native(row.activity_key()), Self::App)
    }

    pub fn resolve<'a>(&self, catalog: &'a crate::SessionCatalog) -> Option<&'a SessionSummary> {
        match self {
            Self::App(id) => catalog.by_app_id(id.get()),
            Self::Native(key) => {
                let mut matches = catalog.at_path(&key.path).filter(|row| key.matches(row));
                let first = matches.next()?;
                matches.next().is_none().then_some(first)
            }
        }
    }

    pub fn matches(&self, row: &SessionSummary) -> bool {
        match self {
            Self::App(id) => id.get() == row.app_session_id,
            Self::Native(key) => key.matches(row),
        }
    }

    pub fn status_key(&self) -> String {
        match self {
            Self::App(id) => id.to_key(),
            Self::Native(key) => format!("session-native:{key:?}"),
        }
    }
}

impl WorkerActivityKey {
    fn matches(&self, row: &SessionSummary) -> bool {
        self.path == row.path
            && self.native.harness == row.harness
            && self.native.profile_id == row.profile_id
            && self.native.project == row.project
            && self.native.id == row.id
    }
}

impl SessionSummary {
    pub fn activity_key(&self) -> WorkerActivityKey {
        WorkerActivityKey {
            path: self.path.clone(),
            native: self.native_identity(),
        }
    }
}

impl crate::SessionCatalog {
    pub fn worker_for_activity(&self, key: &WorkerActivityKey) -> Option<&SessionSummary> {
        self.at_path(&key.path).find(|row| key.matches(row))
    }
}

pub fn agent_activity_key(path: &std::path::Path) -> String {
    crate::normalize_session_path(path)
        .to_string_lossy()
        .into_owned()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentOutcome {
    Complete,
    Failed,
    Incomplete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentLifecycle {
    #[allow(dead_code)]
    NeedsInput,
    Working,
    Unknown,
    Completed(AgentOutcome),
}

impl AgentLifecycle {
    pub fn has_active_work(self) -> bool {
        matches!(self, Self::Working | Self::NeedsInput)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentToolActivity {
    pub name: String,
    pub target: String,
    pub failed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentActivity {
    pub session_id: String,
    pub session_path: PathBuf,
    pub role: String,
    pub activity: String,
    pub lifecycle: AgentLifecycle,
    pub explicit_outcome: bool,
    pub current_tool: Option<AgentToolActivity>,
    pub recent_tool: Option<AgentToolActivity>,
    pub tool_call_count: usize,
    pub limited: bool,
    pub usage: UsageSummary,
    pub started: SystemTime,
    pub ended: Option<SystemTime>,
    pub elapsed: Option<Duration>,
}

impl AgentActivity {
    pub fn limited_fallback(session: &SessionSummary) -> Self {
        Self {
            session_id: session.id.clone(),
            session_path: session.path.clone(),
            role: role_label(&session.title),
            activity: bounded(&session.first_user_message, MAX_ACTIVITY_CHARS),
            lifecycle: if session.is_running {
                AgentLifecycle::Working
            } else {
                AgentLifecycle::Completed(AgentOutcome::Complete)
            },
            explicit_outcome: false,
            current_tool: None,
            recent_tool: None,
            tool_call_count: 0,
            limited: true,
            usage: session.usage,
            started: session.modified,
            ended: (!session.is_running).then_some(session.modified),
            elapsed: None,
        }
    }

    pub fn from_worker_outcome(session: &SessionSummary, outcome: Option<AgentOutcome>) -> Self {
        let mut activity = Self::limited_fallback(session);
        activity.lifecycle = outcome.map_or(AgentLifecycle::Unknown, AgentLifecycle::Completed);
        activity.explicit_outcome = outcome.is_some();
        activity.ended = outcome.map(|_| session.modified);
        activity
    }

    pub fn from_worker_status(
        session: &SessionSummary,
        status: WorkerStatus,
        has_output: bool,
    ) -> Self {
        let mut activity = Self::limited_fallback(session);
        activity.lifecycle = match status {
            WorkerStatus::Pending | WorkerStatus::Running => AgentLifecycle::Working,
            WorkerStatus::NeedsInput => AgentLifecycle::NeedsInput,
            WorkerStatus::Idle if has_output => AgentLifecycle::Completed(AgentOutcome::Complete),
            WorkerStatus::Idle => AgentLifecycle::Unknown,
            WorkerStatus::Failed => AgentLifecycle::Completed(AgentOutcome::Failed),
            WorkerStatus::Stopped => AgentLifecycle::Completed(AgentOutcome::Incomplete),
        };
        activity.explicit_outcome = matches!(activity.lifecycle, AgentLifecycle::Completed(_));
        if matches!(activity.lifecycle, AgentLifecycle::Completed(_)) {
            activity.ended = Some(session.modified);
        }
        activity
    }

    pub fn from_native_child(
        session_id: String,
        session_path: PathBuf,
        title: &str,
        is_running: bool,
        outcome: Option<&str>,
    ) -> Self {
        let now = SystemTime::now();
        let lifecycle = if is_running {
            AgentLifecycle::Working
        } else {
            match outcome {
                Some("complete") => AgentLifecycle::Completed(AgentOutcome::Complete),
                Some("failed") => AgentLifecycle::Completed(AgentOutcome::Failed),
                Some("incomplete") => AgentLifecycle::Completed(AgentOutcome::Incomplete),
                _ => AgentLifecycle::Completed(AgentOutcome::Incomplete),
            }
        };
        let terminal = matches!(lifecycle, AgentLifecycle::Completed(_));
        let explicit_outcome = matches!(outcome, Some("complete" | "failed" | "incomplete"));
        Self {
            session_id,
            session_path,
            role: role_label(title),
            activity: String::new(),
            lifecycle,
            explicit_outcome,
            current_tool: None,
            recent_tool: None,
            tool_call_count: 0,
            limited: true,
            usage: UsageSummary::default(),
            started: now,
            ended: terminal.then_some(now),
            elapsed: None,
        }
    }
}

#[derive(Default)]
pub struct ActivityBuilder {
    tools: HashMap<String, AgentToolActivity>,
    outstanding_tool_ids: Vec<String>,
    recent_tool: Option<AgentToolActivity>,
    tool_call_count: usize,
    outcome: Option<AgentOutcome>,
    terminal_time: Option<SystemTime>,
}

impl ActivityBuilder {
    pub fn observe_entry(&mut self, entry: &Value) {
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
    pub fn finish(
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
        let explicit_outcome = self.outcome.is_some();
        let lifecycle = if is_running {
            AgentLifecycle::Working
        } else if let Some(outcome) = self.outcome {
            AgentLifecycle::Completed(outcome)
        } else if limited {
            AgentLifecycle::Completed(AgentOutcome::Incomplete)
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
            explicit_outcome,
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

pub fn parse_iso_timestamp(value: &str) -> Option<SystemTime> {
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
