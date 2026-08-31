mod persistence;

pub(crate) use persistence::{ProjectStore, allocate_session_id, load_registry, save_registry};

use std::path::{Path, PathBuf};

fn is_excluded(excluded_projects: &[PathBuf], project: &Path) -> bool {
    excluded_projects.iter().any(|excluded| excluded == project)
}

pub(crate) fn add_unique(projects: &mut Vec<PathBuf>, project: PathBuf) -> bool {
    if projects.iter().any(|known| known == &project) {
        return false;
    }
    projects.push(project);
    true
}

pub(crate) fn add_visible(
    projects: &mut Vec<PathBuf>,
    excluded_projects: &[PathBuf],
    project: PathBuf,
) -> bool {
    !is_excluded(excluded_projects, &project) && add_unique(projects, project)
}

pub(crate) fn restore(excluded_projects: &mut Vec<PathBuf>, project: &Path) -> bool {
    let previous_len = excluded_projects.len();
    excluded_projects.retain(|excluded| excluded != project);
    excluded_projects.len() != previous_len
}

pub(crate) fn select(
    projects: &mut Vec<PathBuf>,
    excluded_projects: &[PathBuf],
    project: PathBuf,
) -> bool {
    if is_excluded(excluded_projects, &project) || projects.first() == Some(&project) {
        return false;
    }
    projects.retain(|known| known != &project);
    projects.insert(0, project);
    true
}

pub(crate) fn remove(
    projects: &mut Vec<PathBuf>,
    excluded_projects: &mut Vec<PathBuf>,
    project: &Path,
) -> bool {
    let previous_len = projects.len();
    projects.retain(|known| known != project);
    if projects.len() == previous_len {
        return false;
    }
    if !is_excluded(excluded_projects, project) {
        excluded_projects.push(project.to_path_buf());
    }
    true
}
