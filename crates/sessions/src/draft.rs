use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use farcaster_contracts::Backend;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DraftSession {
    pub id: String,
    #[serde(default)]
    pub app_session_id: i64,
    // No selection while a draft waits for the user to choose a backend.
    #[serde(with = "draft_backend")]
    pub harness: Option<Backend>,
    #[serde(default)]
    pub profile_id: Option<String>,
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
    pub fn fresh(harness: Option<Backend>, project: PathBuf) -> Self {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let id = format!("draft-{}-{}", elapsed.as_nanos(), std::process::id());
        let created_ms = elapsed.as_millis().try_into().unwrap_or(u64::MAX);
        Self::new(harness, id, 0, project, created_ms)
    }

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
            profile_id: None,
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
        if !self.can_change_project() || (self.harness == harness && self.profile_id.is_none()) {
            return false;
        }
        self.harness = harness;
        self.profile_id = None;
        true
    }

    pub fn change_profile(&mut self, harness: Backend, profile_id: String) -> bool {
        if !self.can_change_project()
            || (self.harness == Some(harness)
                && self.profile_id.as_deref() == Some(profile_id.as_str()))
        {
            return false;
        }
        self.harness = Some(harness);
        self.profile_id = Some(profile_id);
        true
    }
}

pub fn submitted_draft_associations(drafts: &[DraftSession]) -> HashMap<String, Option<PathBuf>> {
    drafts
        .iter()
        .filter(|draft| draft.submitted)
        .map(|draft| (draft.id.clone(), draft.session_path.clone()))
        .collect()
}

pub fn sync_materialized_draft(
    drafts: &mut Vec<DraftSession>,
    id: &str,
    app_session_id: i64,
    project: &Path,
    harness: Option<Backend>,
) -> bool {
    if drafts.iter().any(|draft| draft.id == id) {
        return false;
    }
    let mut draft = DraftSession::with_id(harness, id.to_owned(), project.to_path_buf());
    draft.app_session_id = app_session_id;
    drafts.insert(0, draft);
    true
}

pub fn update_persisted_submission(
    drafts: &mut [DraftSession],
    id: &str,
    session: Option<&Path>,
) -> bool {
    let Some(session) = session else {
        return false;
    };
    let Some(draft) = drafts.iter_mut().find(|draft| draft.id == id) else {
        return false;
    };
    let mut changed = false;
    if !draft.submitted {
        draft.submitted = true;
        changed = true;
    }
    if draft.session_path.is_none() {
        draft.session_path = Some(session.to_path_buf());
        changed = true;
    }
    changed
}

pub fn establish_submission(
    submitted_drafts: &mut HashMap<String, Option<PathBuf>>,
    target: &str,
    accepted: bool,
    session: Option<PathBuf>,
) -> Option<String> {
    let id = accepted
        .then(|| target.strip_prefix("draft:").filter(|id| !id.is_empty()))
        .flatten()?
        .to_owned();
    let association = submitted_drafts.entry(id.clone()).or_default();
    if association.is_none() {
        *association = session;
    }
    Some(id)
}

pub fn fill_session_association(
    submitted_drafts: &mut HashMap<String, Option<PathBuf>>,
    target: &str,
    session: Option<&Path>,
) -> Option<PathBuf> {
    let id = target.strip_prefix("draft:").filter(|id| !id.is_empty())?;
    let association = submitted_drafts.get_mut(id)?;
    if association.is_none() {
        *association = session.map(Path::to_path_buf);
    }
    association.clone()
}

pub fn reconciliation_candidates<'a>(
    submitted_drafts: &HashMap<String, Option<PathBuf>>,
    discovered_paths: impl Iterator<Item = &'a Path>,
) -> Vec<(String, PathBuf)> {
    let discovered_paths = discovered_paths.collect::<Vec<_>>();
    submitted_drafts
        .iter()
        .filter_map(|(id, path)| {
            let path = path.as_ref()?;
            discovered_paths
                .contains(&path.as_path())
                .then(|| (id.clone(), path.clone()))
        })
        .collect()
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
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

#[cfg(test)]
#[path = "draft_tests.rs"]
mod tests;
