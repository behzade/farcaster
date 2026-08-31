use std::path::PathBuf;

use crate::{app::infrastructure::persistence::StateStore, projects};

pub(in crate::app) fn new_draft(project: PathBuf) -> Result<projects::DraftSession, String> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let id = format!("draft-{}-{}", elapsed.as_nanos(), std::process::id());
    let created_ms = elapsed.as_millis().try_into().unwrap_or(u64::MAX);
    let mut store = StateStore::open()?;
    let app_session_id = projects::allocate_session_id(&mut store, &id, created_ms)?;
    Ok(projects::DraftSession::new(
        id,
        app_session_id,
        project,
        created_ms,
    ))
}

pub(in crate::app) fn load() -> Result<projects::Registry, String> {
    let mut store = StateStore::open()?;
    let registry = projects::load_registry(&store)?;
    if registry == projects::Registry::default() {
        let legacy_path = legacy_registry_path()?;
        if legacy_path.exists() {
            let legacy = projects::load_legacy(&legacy_path)?;
            projects::save_registry(&mut store, &legacy)?;
            return Ok(legacy);
        }
    }
    Ok(registry)
}

pub(in crate::app) fn save(registry: &projects::Registry) -> Result<(), String> {
    projects::save_registry(&mut StateStore::open()?, registry)
}

pub(in crate::app) fn load_app_session_order() -> Result<Vec<i64>, String> {
    StateStore::open()?.load_app_session_order()
}

pub(in crate::app) fn save_app_session_order(order: &[i64]) -> Result<(), String> {
    StateStore::open()?.save_app_session_order(order)
}

pub(crate) fn most_recent() -> Option<PathBuf> {
    load().ok()?.projects.into_iter().next()
}

fn legacy_registry_path() -> Result<PathBuf, String> {
    crate::app::infrastructure::paths::data_dir().map(|root| root.join("projects.json"))
}
