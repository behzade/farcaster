use std::path::PathBuf;

use crate::projects;

pub(in crate::app) fn load() -> Result<projects::ProjectList, String> {
    let mut store = crate::app::persistence::open()?;
    let projects = projects::load_projects(&*store)?;
    if projects == projects::ProjectList::default() && store.load_drafts()?.is_empty() {
        let legacy_path = legacy_registry_path()?;
        if legacy_path.exists() {
            let legacy = projects::load_legacy(&legacy_path)?;
            store.save_registry(&legacy)?;
            return Ok(projects::ProjectList {
                projects: legacy.projects,
                excluded_projects: legacy.excluded_projects,
            });
        }
    }
    Ok(projects)
}

pub(in crate::app) fn save(projects: &projects::ProjectList) -> Result<(), String> {
    projects::save_projects(&mut *crate::app::persistence::open()?, projects)
}

pub(in crate::app) fn load_app_session_order() -> Result<Vec<i64>, String> {
    crate::app::persistence::open()?.load_app_session_order()
}

pub(in crate::app) fn save_app_session_order(order: &[i64]) -> Result<(), String> {
    crate::app::persistence::open()?.save_app_session_order(order)
}

pub(crate) fn most_recent() -> Option<PathBuf> {
    load().ok()?.projects.into_iter().next()
}

fn legacy_registry_path() -> Result<PathBuf, String> {
    crate::app::infrastructure::paths::data_dir().map(|root| root.join("projects.json"))
}
