use std::{collections::HashSet, fs, path::Path};

use super::Registry;

pub(crate) fn is_temporary_project(path: &Path) -> bool {
    path_is_within(path, &std::env::temp_dir())
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    path.starts_with(root)
}

pub(crate) fn load_legacy(path: &Path) -> Result<Registry, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_path_filter_is_scoped_to_the_configured_root() -> Result<(), std::io::Error> {
        let root = tempfile::tempdir()?;
        let project = root.path().join("project");
        fs::create_dir(&project)?;

        assert!(path_is_within(&project, root.path()));
        assert!(!path_is_within(
            root.path().parent().unwrap_or(root.path()),
            root.path()
        ));
        Ok(())
    }
}
