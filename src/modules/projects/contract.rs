use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StartupTrust {
    Ready,
    Prompt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrustChoice {
    TrustProject,
    TrustParent,
    DistrustProject,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TrustOption {
    pub label: String,
    pub choice: TrustChoice,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AppliedTrust {
    pub trusted: bool,
    pub saved_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DraftSession {
    pub id: String,
    #[serde(default)]
    pub app_session_id: i64,
    pub project: PathBuf,
    pub created_ms: u64,
    #[serde(default)]
    pub submitted: bool,
    #[serde(default)]
    pub session_path: Option<PathBuf>,
    #[serde(default)]
    pub title: Option<String>,
}

impl DraftSession {
    pub(crate) fn new(id: String, app_session_id: i64, project: PathBuf, created_ms: u64) -> Self {
        Self {
            id,
            app_session_id,
            project,
            created_ms,
            submitted: false,
            session_path: None,
            title: None,
        }
    }

    pub(crate) fn with_id(id: String, project: PathBuf) -> Self {
        Self::new(id, 0, project, current_time_ms())
    }

    pub(crate) const fn can_change_project(&self) -> bool {
        !self.submitted && self.session_path.is_none()
    }

    pub(crate) fn change_project(&mut self, project: PathBuf) -> bool {
        if !self.can_change_project() || self.project == project {
            return false;
        }
        self.project = project;
        true
    }
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Registry {
    pub projects: Vec<PathBuf>,
    #[serde(default, skip_serializing)]
    pub excluded_projects: Vec<PathBuf>,
    pub drafts: Vec<DraftSession>,
}
