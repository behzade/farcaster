use std::{collections::HashMap, path::PathBuf, time::SystemTime};

use serde_json::Value;

use super::activity::AgentActivity;

pub(crate) const RUNNING_ACTIVITY_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(30 * 60);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct UsageSummary {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
    pub cost_micros: u64,
}

impl UsageSummary {
    pub(crate) fn add(&mut self, other: Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
        self.total = self.total.saturating_add(other.total);
        self.cost_micros = self.cost_micros.saturating_add(other.cost_micros);
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SessionTarget {
    pub harness: String,
    pub id: String,
    pub path: PathBuf,
}

impl SessionTarget {
    pub(crate) fn pi(path: PathBuf) -> Self {
        Self {
            harness: "pi".into(),
            id: path.to_string_lossy().into_owned(),
            path,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionImport {
    pub id: String,
    pub harness: String,
    pub path: PathBuf,
    pub project: PathBuf,
    pub title: String,
    pub first_user_message: String,
    pub timestamp: String,
    pub parent_session: Option<String>,
    pub modified: SystemTime,
    pub message_count: usize,
    pub usage: UsageSummary,
    pub archived: bool,
    pub is_running: bool,
    pub search: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionSummary {
    pub id: String,
    pub app_session_id: i64,
    pub harness: String,
    pub path: PathBuf,
    pub project: PathBuf,
    pub title: String,
    pub first_user_message: String,
    pub timestamp: String,
    pub parent_session: Option<String>,
    pub modified: SystemTime,
    pub message_count: usize,
    pub usage: UsageSummary,
    pub archived: bool,
    pub is_running: bool,
    pub model: Option<(String, String)>,
    pub thinking_level: Option<String>,
    pub(super) search: String,
}

impl SessionSummary {
    pub(crate) fn import(value: SessionImport) -> Self {
        let is_running = value.is_running
            && SystemTime::now()
                .duration_since(value.modified)
                .unwrap_or_default()
                <= RUNNING_ACTIVITY_TIMEOUT;
        Self {
            id: value.id,
            app_session_id: 0,
            harness: value.harness,
            path: value.path,
            project: value.project,
            title: value.title,
            first_user_message: value.first_user_message,
            timestamp: value.timestamp,
            parent_session: value.parent_session,
            modified: value.modified,
            message_count: value.message_count,
            usage: value.usage,
            archived: value.archived,
            is_running,
            model: None,
            thinking_level: None,
            search: value.search.to_lowercase(),
        }
    }

    pub(crate) fn target(&self) -> SessionTarget {
        SessionTarget {
            harness: self.harness.clone(),
            id: self.id.clone(),
            path: self.path.clone(),
        }
    }

    pub(crate) fn with_app_session_id(mut self, app_session_id: i64) -> Self {
        self.app_session_id = app_session_id;
        self
    }

    pub(crate) fn search_text(&self) -> &str {
        &self.search
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SessionDiscovery {
    pub sessions: Vec<SessionSummary>,
    pub activities: HashMap<String, AgentActivity>,
    pub exhaustive: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum SessionWatchEvent {
    CatalogChanged,
    Activity(Vec<PathBuf>),
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransferMember {
    pub path: PathBuf,
    pub id: String,
    pub parent_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionTransfer {
    pub root: PathBuf,
    pub paths: HashMap<PathBuf, PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RestoredQuestion {
    pub id: String,
    pub title: String,
    pub options: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct LoadedHistory {
    pub messages: Vec<Value>,
    pub model: Option<(String, String)>,
    pub thinking_level: Option<String>,
    pub pending_question: Option<RestoredQuestion>,
}
