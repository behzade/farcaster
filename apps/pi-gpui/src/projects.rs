//! Small persistent registry for projects that can start a native Pi session.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DraftSession {
    pub id: String,
    pub project: PathBuf,
    pub created_ms: u64,
}

impl DraftSession {
    pub(crate) fn new(project: PathBuf) -> Self {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let created_ms = elapsed.as_millis().try_into().unwrap_or(u64::MAX);
        Self {
            id: format!("draft-{}-{}", elapsed.as_nanos(), std::process::id()),
            project,
            created_ms,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Registry {
    pub projects: Vec<PathBuf>,
    pub drafts: Vec<DraftSession>,
}

pub(crate) fn load() -> Result<Registry, String> {
    load_from(&registry_path()?)
}

pub(crate) fn save(registry: &Registry) -> Result<(), String> {
    save_to(&registry_path()?, registry)
}

pub(crate) fn add_unique(projects: &mut Vec<PathBuf>, project: PathBuf) -> bool {
    if projects.iter().any(|known| known == &project) {
        return false;
    }
    projects.push(project);
    true
}

fn registry_path() -> Result<PathBuf, String> {
    let root = if let Some(root) = std::env::var_os("PI_CODING_AGENT_DIR") {
        PathBuf::from(root)
    } else {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "HOME is not set and PI_CODING_AGENT_DIR is not set".to_owned())?
            .join(".pi/agent")
    };
    let root = if root.is_absolute() {
        root
    } else {
        std::env::current_dir()
            .map_err(|error| format!("resolve project registry directory: {error}"))?
            .join(root)
    };
    Ok(root.join("gui-state.json"))
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
            (draft.project.is_dir() && seen_drafts.insert(draft.id.clone())).then_some(draft)
        })
        .collect();
    Ok(Registry { projects, drafts })
}

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
                drafts: vec![DraftSession {
                    id: "draft-one".into(),
                    project: first.clone(),
                    created_ms: 1,
                }],
            },
        )?;

        assert_eq!(
            load_from(&path)?,
            Registry {
                projects: vec![first.canonicalize()?, second.canonicalize()?],
                drafts: vec![DraftSession {
                    id: "draft-one".into(),
                    project: first.canonicalize()?,
                    created_ms: 1,
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
                drafts: vec![DraftSession {
                    id: "gone".into(),
                    project: temp.path().join("gone"),
                    created_ms: 1,
                }],
            },
        )?;

        assert_eq!(load_from(&path)?, Registry::default());
        Ok(())
    }
}
