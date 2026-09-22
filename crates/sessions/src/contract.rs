use farcaster_contracts::Backend;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::SystemTime};

use serde_json::Value;

use super::activity::AgentActivity;

pub const RUNNING_ACTIVITY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UsageSummary {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
    pub cost_micros: u64,
}

impl UsageSummary {
    pub fn add(&mut self, other: Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
        self.total = self.total.saturating_add(other.total);
        self.cost_micros = self.cost_micros.saturating_add(other.cost_micros);
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SessionTarget {
    pub harness: Backend,
    pub id: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionImport {
    pub id: String,
    pub harness: Backend,
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
pub struct SessionSummary {
    pub id: String,
    pub app_session_id: i64,
    pub harness: Backend,
    pub path: PathBuf,
    pub project: PathBuf,
    pub title: String,
    pub first_user_message: String,
    pub timestamp: String,
    pub parent_session: Option<String>,
    pub parent_harness: Option<Backend>,
    /// Resolved application identity; a parent need not share this session's project.
    pub parent_app_session_id: Option<i64>,
    pub modified: SystemTime,
    pub message_count: usize,
    pub usage: UsageSummary,
    pub archived: bool,
    pub is_running: bool,
    pub model: Option<(String, String)>,
    pub thinking_level: Option<String>,
    pub search: Arc<str>,
}

impl SessionSummary {
    pub fn import(value: SessionImport) -> Self {
        let is_running = value.is_running
            && SystemTime::now()
                .duration_since(value.modified)
                .unwrap_or_default()
                <= RUNNING_ACTIVITY_TIMEOUT;
        let parent_harness = value.parent_session.as_ref().map(|_| value.harness);
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
            parent_harness,
            parent_app_session_id: None,
            modified: value.modified,
            message_count: value.message_count,
            usage: value.usage,
            archived: value.archived,
            is_running,
            model: None,
            thinking_level: None,
            search: value.search.to_lowercase().into(),
        }
    }

    pub fn target(&self) -> SessionTarget {
        SessionTarget {
            harness: self.harness,
            id: self.id.clone(),
            path: self.path.clone(),
        }
    }

    pub fn with_app_session_id(mut self, app_session_id: i64) -> Self {
        self.app_session_id = app_session_id;
        self
    }

    pub fn search_text(&self) -> &str {
        &self.search
    }

    #[cfg(any(test, feature = "test-support"))]
    #[allow(clippy::too_many_arguments)]
    pub fn from_cached(
        id: String,
        path: PathBuf,
        project: PathBuf,
        title: String,
        first_user_message: String,
        timestamp: String,
        parent_session: Option<String>,
        modified: SystemTime,
        message_count: usize,
        usage: UsageSummary,
        archived: bool,
        is_running: bool,
        search: String,
    ) -> Self {
        Self::from_cached_for_harness(
            id,
            Backend::Pi,
            path,
            project,
            title,
            first_user_message,
            timestamp,
            parent_session,
            modified,
            message_count,
            usage,
            archived,
            is_running,
            search,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_cached_for_harness(
        id: String,
        harness: Backend,
        path: PathBuf,
        project: PathBuf,
        title: String,
        first_user_message: String,
        timestamp: String,
        parent_session: Option<String>,
        modified: SystemTime,
        message_count: usize,
        usage: UsageSummary,
        archived: bool,
        is_running: bool,
        search: String,
    ) -> Self {
        let is_running = is_running
            && SystemTime::now()
                .duration_since(modified)
                .unwrap_or_default()
                <= RUNNING_ACTIVITY_TIMEOUT;
        let parent_harness = parent_session.as_ref().map(|_| harness);
        Self {
            id,
            app_session_id: 0,
            harness,
            path,
            project,
            title,
            first_user_message,
            timestamp,
            parent_session,
            parent_harness,
            parent_app_session_id: None,
            modified,
            message_count,
            usage,
            archived,
            is_running,
            model: None,
            thinking_level: None,
            search: search.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionDiscovery {
    pub sessions: Vec<SessionSummary>,
    pub activities: HashMap<String, AgentActivity>,
    #[allow(
        dead_code,
        reason = "Adapters report scan completeness alongside discovered sessions."
    )]
    pub exhaustive: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferMember {
    pub path: PathBuf,
    pub id: String,
    pub parent_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionTransfer {
    pub root: PathBuf,
    pub paths: HashMap<PathBuf, PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoredQuestion {
    pub id: String,
    pub title: String,
    pub options: Vec<String>,
}

#[derive(Debug)]
pub struct LoadedHistory {
    pub messages: Vec<Value>,
    pub model: Option<(String, String)>,
    pub thinking_level: Option<String>,
    pub pending_question: Option<RestoredQuestion>,
    /// Authoritative backend evidence for Farcaster submission IDs. Absence
    /// means this history source cannot prove whether a missing ID is pending.
    pub prompt_deliveries: Option<PromptDeliveryReconciliation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptDeliveryReconciliation {
    pub delivered: Vec<String>,
    pub pending: Vec<String>,
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;
