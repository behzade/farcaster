use farcaster_contracts::Backend;
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupTrust {
    Ready,
    Prompt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustChoice {
    TrustProject,
    TrustParent,
    DistrustProject,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustOption {
    pub label: String,
    pub choice: TrustChoice,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppliedTrust {
    pub trusted: bool,
    pub saved_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DraftSession {
    pub id: String,
    #[serde(default)]
    pub app_session_id: i64,
    // No selection while a draft waits for the user to choose a backend.
    #[serde(with = "draft_backend")]
    pub harness: Option<Backend>,
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
    pub fn new(
        harness: Option<Backend>,
        id: String,
        app_session_id: i64,
        project: PathBuf,
        created_ms: u64,
    ) -> Self {
        Self {
            id,
            app_session_id,
            harness,
            project,
            created_ms,
            submitted: false,
            session_path: None,
            title: None,
        }
    }

    pub fn with_id(harness: Option<Backend>, id: String, project: PathBuf) -> Self {
        Self::new(harness, id, 0, project, current_time_ms())
    }

    pub const fn can_change_project(&self) -> bool {
        !self.submitted && self.session_path.is_none()
    }

    pub fn change_project(&mut self, project: PathBuf) -> bool {
        if !self.can_change_project() || self.project == project {
            return false;
        }
        self.project = project;
        true
    }

    pub fn change_harness(&mut self, harness: Option<Backend>) -> bool {
        if !self.can_change_project() || self.harness == harness {
            return false;
        }
        self.harness = harness;
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
pub struct Registry {
    pub projects: Vec<PathBuf>,
    #[serde(default, skip_serializing)]
    pub excluded_projects: Vec<PathBuf>,
    pub drafts: Vec<DraftSession>,
}

mod draft_backend {
    use farcaster_contracts::Backend;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        backend: &Option<Backend>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(backend.map(Backend::as_str).unwrap_or(""))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Backend>, D::Error> {
        let value = Option::<String>::deserialize(deserializer)?;
        match value.as_deref() {
            None | Some("") => Ok(None),
            Some(value) => serde_json::from_value(serde_json::Value::String(value.to_owned()))
                .map(Some)
                .map_err(serde::de::Error::custom),
        }
    }
}
