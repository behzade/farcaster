use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::state::StateStore;

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

pub(crate) fn most_recent() -> Option<PathBuf> {
    load().ok()?.projects.into_iter().next()
}

fn registry_path() -> Result<PathBuf, String> {
    crate::app_paths::data_dir().map(|root| root.join("projects.json"))
}

fn load_from(path: &Path) -> Result<Registry, String> {
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
fn save_to(path: &Path, registry: &Registry) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn registry_round_trips_unique_existing_projects() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir(&first)?;
        fs::create_dir(&second)?;
        let path = temp.path().join("state/projects.json");

        save_to(
            &path,
            &Registry {
                projects: vec![first.clone(), second.clone(), first.clone()],
                excluded_projects: Vec::new(),
                drafts: vec![DraftSession {
                    id: "draft-one".into(),
                    app_session_id: 7,
                    project: first.clone(),
                    created_ms: 1,
                    submitted: true,
                    session_path: Some(second.clone()),
                    title: None,
                }],
            },
        )?;

        assert_eq!(
            load_from(&path)?,
            Registry {
                projects: vec![first.canonicalize()?, second.canonicalize()?],
                excluded_projects: Vec::new(),
                drafts: vec![DraftSession {
                    id: "draft-one".into(),
                    app_session_id: 7,
                    project: first.canonicalize()?,
                    created_ms: 1,
                    submitted: true,
                    session_path: Some(second.canonicalize()?),
                    title: None,
                }],
            }
        );
        Ok(())
    }

    #[test]
    fn registry_ignores_projects_that_no_longer_exist() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let path = temp.path().join("projects.json");
        save_to(
            &path,
            &Registry {
                projects: vec![temp.path().join("gone")],
                excluded_projects: Vec::new(),
                drafts: vec![DraftSession {
                    id: "gone".into(),
                    app_session_id: 8,
                    project: temp.path().join("gone"),
                    created_ms: 1,
                    submitted: false,
                    session_path: None,
                    title: None,
                }],
            },
        )?;

        assert_eq!(load_from(&path)?, Registry::default());
        Ok(())
    }

    #[test]
    fn linked_worktrees_are_distinct_projects_that_can_be_selected_and_restored()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let repository = temp.path().join("repository");
        let worktree = temp.path().join("worktree");
        let worktree_git_dir = repository.join(".git/worktrees/feature");
        fs::create_dir_all(&worktree_git_dir)?;
        fs::create_dir_all(&worktree)?;
        fs::write(worktree_git_dir.join("commondir"), "../..\n")?;
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", worktree_git_dir.display()),
        )?;
        fs::create_dir_all(repository.join(".git"))?;

        let repository = repository.canonicalize()?;
        let worktree = worktree.canonicalize()?;
        let mut projects = vec![repository.clone()];
        let mut excluded_projects = Vec::new();
        assert!(add_unique(&mut projects, worktree.clone()));
        assert!(!add_unique(&mut projects, worktree.clone()));
        assert_eq!(projects, vec![repository.clone(), worktree.clone()]);

        assert!(select(&mut projects, &excluded_projects, worktree.clone()));
        assert_eq!(projects, vec![worktree.clone(), repository.clone()]);
        assert!(remove(&mut projects, &mut excluded_projects, &worktree));
        assert!(restore(&mut excluded_projects, &worktree));
        assert!(add_visible(
            &mut projects,
            &excluded_projects,
            worktree.clone()
        ));
        assert_eq!(projects, vec![repository, worktree]);
        Ok(())
    }

    #[test]
    fn selecting_a_project_moves_it_to_the_front_unless_it_was_removed() {
        let first = PathBuf::from("/first");
        let second = PathBuf::from("/second");
        let removed = PathBuf::from("/removed");
        let mut projects = vec![first.clone(), second.clone()];

        assert!(select(&mut projects, &[], second.clone()));
        assert_eq!(projects, vec![second.clone(), first]);
        assert!(!select(&mut projects, &[], second));
        assert!(!select(
            &mut projects,
            std::slice::from_ref(&removed),
            removed.clone()
        ));
    }

    #[test]
    fn removing_a_project_only_changes_registered_matches() {
        let first = PathBuf::from("/first");
        let second = PathBuf::from("/second");
        let mut projects = vec![first.clone(), second.clone()];
        let mut excluded_projects = Vec::new();

        assert!(remove(&mut projects, &mut excluded_projects, &first));
        assert_eq!(projects, vec![second]);
        assert_eq!(excluded_projects, vec![first.clone()]);
        assert!(!remove(&mut projects, &mut excluded_projects, &first));
        assert!(restore(&mut excluded_projects, &first));
        assert!(excluded_projects.is_empty());
    }

    #[test]
    fn only_unsubmitted_drafts_can_change_project() {
        let mut draft = DraftSession::new("draft".into(), 1, PathBuf::from("/first"), 1);
        assert!(draft.change_project(PathBuf::from("/second")));
        assert_eq!(draft.project, PathBuf::from("/second"));
        assert!(!draft.change_project(PathBuf::from("/second")));

        draft.submitted = true;
        assert!(!draft.change_project(PathBuf::from("/third")));
        assert_eq!(draft.project, PathBuf::from("/second"));
    }

    #[test]
    fn old_registry_drafts_decode_as_unsubmitted() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let project = temp.path().join("project");
        fs::create_dir(&project)?;
        let path = temp.path().join("projects.json");
        fs::write(
            &path,
            serde_json::json!({
                "projects": [project.clone()],
                "drafts": [{"id": "legacy", "project": project, "created_ms": 3}]
            })
            .to_string(),
        )?;

        let registry = load_from(&path)?;
        assert_eq!(registry.drafts.len(), 1);
        assert!(!registry.drafts[0].submitted);
        assert_eq!(registry.drafts[0].session_path, None);
        Ok(())
    }
}
