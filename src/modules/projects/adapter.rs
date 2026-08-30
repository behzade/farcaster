use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::state::StateStore;

use super::{DraftSession, Registry};

pub(crate) fn new_draft(project: PathBuf) -> Result<DraftSession, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let id = format!("draft-{}-{}", elapsed.as_nanos(), std::process::id());
    draft_with_id_at(
        id,
        project,
        elapsed.as_millis().try_into().unwrap_or(u64::MAX),
    )
}

fn draft_with_id_at(id: String, project: PathBuf, created_ms: u64) -> Result<DraftSession, String> {
    let mut store = StateStore::open()?;
    let app_session_id = store.allocate_app_session_id(&id, created_ms)?;
    Ok(DraftSession::new(id, app_session_id, project, created_ms))
}

pub(crate) fn load() -> Result<Registry, String> {
    let mut store = StateStore::open()?;
    let registry = store.load_registry()?;
    if registry == Registry::default() {
        let legacy_path = registry_path()?;
        if legacy_path.exists() {
            let legacy = load_from(&legacy_path)?;
            store.save_registry(&legacy)?;
            return Ok(legacy);
        }
    }
    Ok(registry)
}

pub(crate) fn save(registry: &Registry) -> Result<(), String> {
    StateStore::open()?.save_registry(registry)
}

pub(crate) fn load_app_session_order() -> Result<Vec<i64>, String> {
    StateStore::open()?.load_app_session_order()
}

pub(crate) fn save_app_session_order(order: &[i64]) -> Result<(), String> {
    StateStore::open()?.save_app_session_order(order)
}

pub(crate) fn most_recent() -> Option<PathBuf> {
    load().ok()?.projects.into_iter().next()
}

fn registry_path() -> Result<PathBuf, String> {
    crate::app_paths::data_dir().map(|root| root.join("projects.json"))
}

pub(super) fn load_from(path: &Path) -> Result<Registry, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Registry::default());
        }
        Err(error) => return Err(format!("read {}: {error}", path.display())),
    };
    let stored = serde_json::from_slice::<Registry>(&bytes)
        .map_err(|error| format!("decode {}: {error}", path.display()))?;
    let mut seen = HashSet::new();
    let projects = stored
        .projects
        .into_iter()
        .filter_map(|project| project.canonicalize().ok())
        .filter(|project| project.is_dir() && seen.insert(project.clone()))
        .collect::<Vec<_>>();
    let mut seen_drafts = HashSet::new();
    let drafts = stored
        .drafts
        .into_iter()
        .filter_map(|mut draft| {
            draft.project = draft.project.canonicalize().ok()?;
            draft.session_path = draft
                .session_path
                .map(|path| crate::sessions::normalize_session_path(&path));
            if draft.session_path.is_none() {
                draft.submitted = false;
            }
            (draft.project.is_dir() && seen_drafts.insert(draft.id.clone())).then_some(draft)
        })
        .collect();
    Ok(Registry {
        projects,
        excluded_projects: Vec::new(),
        drafts,
    })
}

#[cfg(test)]
pub(super) fn save_to(path: &Path, registry: &Registry) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("project registry has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| format!("create {}: {error}", parent.display()))?;
    let bytes = serde_json::to_vec_pretty(registry)
        .map_err(|error| format!("encode project registry: {error}"))?;
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&temporary, bytes)
        .map_err(|error| format!("write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|error| {
        format!(
            "replace {} with {}: {error}",
            path.display(),
            temporary.display()
        )
    })
}
