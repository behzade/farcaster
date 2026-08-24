//! Filesystem event source for repository working-copy and metadata changes.

use std::{
    fs,
    path::{Path, PathBuf},
};

use async_channel::Receiver;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

use super::{RepositoryKind, RepositoryLocation};

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum RepositoryWatchEvent {
    Changed,
    Failed(String),
}

pub(crate) struct RepositoryWatcher {
    _watcher: RecommendedWatcher,
}

impl RepositoryWatcher {
    pub(crate) fn start(
        location: &RepositoryLocation,
    ) -> Result<(Self, Receiver<RepositoryWatchEvent>), String> {
        Self::start_targets(watch_targets(location)?, repository_event)
    }

    pub(crate) fn start_discovery(
        project: &Path,
    ) -> Result<(Self, Receiver<RepositoryWatchEvent>), String> {
        Self::start_targets(discovery_targets(project)?, discovery_event)
    }

    fn start_targets(
        targets: Vec<WatchTarget>,
        classify: fn(notify::Result<Event>) -> Option<RepositoryWatchEvent>,
    ) -> Result<(Self, Receiver<RepositoryWatchEvent>), String> {
        let (sender, receiver) = async_channel::unbounded();
        let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
            if let Some(event) = classify(result) {
                let _ = sender.try_send(event);
            }
        })
        .map_err(|error| format!("create repository watcher: {error}"))?;
        for target in targets {
            watcher
                .watch(&target.path, target.mode)
                .map_err(|error| format!("watch {}: {error}", target.path.display()))?;
        }
        Ok((Self { _watcher: watcher }, receiver))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WatchTarget {
    path: PathBuf,
    mode: RecursiveMode,
}

fn repository_event(result: notify::Result<Event>) -> Option<RepositoryWatchEvent> {
    match result {
        Ok(event) if matches!(event.kind, EventKind::Access(_)) => None,
        Ok(_) => Some(RepositoryWatchEvent::Changed),
        Err(error) => watcher_failure(error),
    }
}

fn discovery_event(result: notify::Result<Event>) -> Option<RepositoryWatchEvent> {
    match result {
        Ok(event) if matches!(event.kind, EventKind::Access(_)) => None,
        Ok(event)
            if event.paths.iter().any(|path| {
                path.components().any(|component| {
                    let component = component.as_os_str();
                    component == ".git" || component == ".jj"
                })
            }) =>
        {
            Some(RepositoryWatchEvent::Changed)
        }
        Ok(_) => None,
        Err(error) => watcher_failure(error),
    }
}

fn watcher_failure(error: notify::Error) -> Option<RepositoryWatchEvent> {
    Some(RepositoryWatchEvent::Failed(format!(
        "watch repository: {error}"
    )))
}

fn discovery_targets(project: &Path) -> Result<Vec<WatchTarget>, String> {
    let project = project.canonicalize().map_err(|error| {
        format!(
            "resolve project watch target {}: {error}",
            project.display()
        )
    })?;
    let mut targets = Vec::new();
    add_target(&mut targets, project.clone(), RecursiveMode::Recursive);
    let mut ancestor = project.parent();
    while let Some(path) = ancestor {
        add_target(
            &mut targets,
            path.to_path_buf(),
            RecursiveMode::NonRecursive,
        );
        ancestor = path.parent();
    }
    Ok(targets)
}

fn watch_targets(location: &RepositoryLocation) -> Result<Vec<WatchTarget>, String> {
    let mut targets = Vec::new();
    add_target(
        &mut targets,
        location.project_root.clone(),
        RecursiveMode::Recursive,
    );
    match location.kind {
        RepositoryKind::Git => add_git_targets(&mut targets, &location.workspace_root)?,
        RepositoryKind::Jujutsu => add_jj_targets(&mut targets, &location.workspace_root)?,
    }
    Ok(targets)
}

fn add_jj_targets(targets: &mut Vec<WatchTarget>, workspace_root: &Path) -> Result<(), String> {
    let marker = workspace_root.join(".jj");
    add_existing_target(targets, marker.clone(), RecursiveMode::Recursive)?;
    let repo = marker.join("repo");
    if repo.is_file() {
        let value = fs::read_to_string(&repo)
            .map_err(|error| format!("read {}: {error}", repo.display()))?;
        let value = value.trim();
        if value.is_empty() {
            return Err(format!(
                "invalid Jujutsu repository pointer: {}",
                repo.display()
            ));
        }
        let shared_repo = resolve_relative(&marker, Path::new(value))?;
        add_existing_target(targets, shared_repo, RecursiveMode::Recursive)?;
    }
    Ok(())
}

fn add_git_targets(targets: &mut Vec<WatchTarget>, workspace_root: &Path) -> Result<(), String> {
    let marker = workspace_root.join(".git");
    if marker.is_dir() {
        return add_existing_target(targets, marker, RecursiveMode::Recursive);
    }
    if !marker.is_file() {
        return Err(format!(
            "Git metadata marker is missing: {}",
            marker.display()
        ));
    }
    add_existing_target(targets, marker.clone(), RecursiveMode::NonRecursive)?;
    let git_dir = resolve_git_dir(&marker)?;
    add_existing_target(targets, git_dir.clone(), RecursiveMode::Recursive)?;
    let common_dir_file = git_dir.join("commondir");
    if common_dir_file.is_file() {
        let value = fs::read_to_string(&common_dir_file)
            .map_err(|error| format!("read {}: {error}", common_dir_file.display()))?;
        let value = value.lines().next().unwrap_or_default().trim();
        if !value.is_empty() {
            let common_dir = resolve_relative(&git_dir, Path::new(value))?;
            add_existing_target(targets, common_dir, RecursiveMode::Recursive)?;
        }
    }
    Ok(())
}

fn resolve_git_dir(marker: &Path) -> Result<PathBuf, String> {
    let value = fs::read_to_string(marker)
        .map_err(|error| format!("read {}: {error}", marker.display()))?;
    let git_dir = value
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("gitdir: "))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| format!("invalid Git metadata pointer: {}", marker.display()))?;
    resolve_relative(
        marker.parent().unwrap_or_else(|| Path::new(".")),
        Path::new(git_dir),
    )
}

fn resolve_relative(base: &Path, path: &Path) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    path.canonicalize()
        .map_err(|error| format!("resolve {}: {error}", path.display()))
}

fn add_existing_target(
    targets: &mut Vec<WatchTarget>,
    path: PathBuf,
    mode: RecursiveMode,
) -> Result<(), String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("resolve watch target {}: {error}", path.display()))?;
    add_target(targets, path, mode);
    Ok(())
}

fn add_target(targets: &mut Vec<WatchTarget>, path: PathBuf, mode: RecursiveMode) {
    if targets.iter().any(|target| {
        target.path == path
            || (target.mode == RecursiveMode::Recursive && path.starts_with(&target.path))
    }) {
        return;
    }
    if mode == RecursiveMode::Recursive {
        targets.retain(|target| !target.path.starts_with(&path));
    }
    targets.push(WatchTarget { path, mode });
}

#[cfg(test)]
mod tests {
    use notify::{
        Event,
        event::{AccessKind, CreateKind},
    };

    use super::*;

    #[test]
    fn ignores_access_and_reports_changes() {
        assert_eq!(
            repository_event(Ok(Event::new(EventKind::Access(AccessKind::Any)))),
            None
        );
        assert_eq!(
            repository_event(Ok(Event::new(EventKind::Create(CreateKind::File)))),
            Some(RepositoryWatchEvent::Changed)
        );
    }

    #[test]
    fn discovery_watches_project_and_ancestors_but_only_accepts_repository_markers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = temp.path().join("parent/project");
        fs::create_dir_all(&project).expect("project");
        let project = project.canonicalize().expect("project");
        let parent = project.parent().expect("parent").to_path_buf();

        let targets = discovery_targets(&project).expect("targets");
        assert!(targets.contains(&WatchTarget {
            path: project.clone(),
            mode: RecursiveMode::Recursive,
        }));
        assert!(targets.contains(&WatchTarget {
            path: parent,
            mode: RecursiveMode::NonRecursive,
        }));
        assert_eq!(
            discovery_event(Ok(
                Event::new(EventKind::Create(CreateKind::Folder)).add_path(project.join(".jj"))
            )),
            Some(RepositoryWatchEvent::Changed)
        );
        assert_eq!(
            discovery_event(Ok(
                Event::new(EventKind::Create(CreateKind::File)).add_path(project.join("file.rs"))
            )),
            None
        );
    }

    #[test]
    fn nested_git_project_watches_project_and_repository_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        let project = workspace.join("app");
        fs::create_dir_all(workspace.join(".git")).expect("git metadata");
        fs::create_dir_all(&project).expect("project");
        let workspace = workspace.canonicalize().expect("workspace");
        let project = project.canonicalize().expect("project");

        let targets = watch_targets(&RepositoryLocation {
            kind: RepositoryKind::Git,
            workspace_root: workspace.clone(),
            project_root: project.clone(),
        })
        .expect("targets");

        assert!(targets.contains(&WatchTarget {
            path: project,
            mode: RecursiveMode::Recursive,
        }));
        assert!(targets.contains(&WatchTarget {
            path: workspace.join(".git"),
            mode: RecursiveMode::Recursive,
        }));
    }

    #[test]
    fn nested_jujutsu_project_watches_project_and_repository_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        let project = workspace.join("app");
        fs::create_dir_all(workspace.join(".jj")).expect("jj metadata");
        fs::create_dir_all(&project).expect("project");
        let workspace = workspace.canonicalize().expect("workspace");
        let project = project.canonicalize().expect("project");

        let targets = watch_targets(&RepositoryLocation {
            kind: RepositoryKind::Jujutsu,
            workspace_root: workspace.clone(),
            project_root: project.clone(),
        })
        .expect("targets");

        assert!(targets.contains(&WatchTarget {
            path: project,
            mode: RecursiveMode::Recursive,
        }));
        assert!(targets.contains(&WatchTarget {
            path: workspace.join(".jj"),
            mode: RecursiveMode::Recursive,
        }));
    }

    #[test]
    fn secondary_jujutsu_workspace_watches_shared_repository_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        let shared_repo = temp.path().join("primary/.jj/repo");
        fs::create_dir_all(workspace.join(".jj")).expect("workspace metadata");
        fs::create_dir_all(&shared_repo).expect("shared metadata");
        fs::write(workspace.join(".jj/repo"), "../../primary/.jj/repo").expect("repo pointer");
        let workspace = workspace.canonicalize().expect("workspace");
        let shared_repo = shared_repo.canonicalize().expect("shared metadata");

        let targets = watch_targets(&RepositoryLocation {
            kind: RepositoryKind::Jujutsu,
            workspace_root: workspace.clone(),
            project_root: workspace,
        })
        .expect("targets");

        assert!(targets.iter().any(|target| {
            target.path == shared_repo && target.mode == RecursiveMode::Recursive
        }));
    }

    #[test]
    fn linked_worktree_watches_common_metadata_that_contains_private_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("worktree");
        let git_dir = temp.path().join("main/.git/worktrees/worktree");
        let common_dir = temp.path().join("main/.git");
        fs::create_dir_all(&workspace).expect("workspace");
        fs::create_dir_all(&git_dir).expect("worktree metadata");
        fs::write(
            workspace.join(".git"),
            format!("gitdir: {}\n", git_dir.display()),
        )
        .expect("git pointer");
        fs::write(git_dir.join("commondir"), "../..\n").expect("common pointer");
        let workspace = workspace.canonicalize().expect("workspace");
        let git_dir = git_dir.canonicalize().expect("worktree metadata");
        let common_dir = common_dir.canonicalize().expect("common metadata");

        let targets = watch_targets(&RepositoryLocation {
            kind: RepositoryKind::Git,
            workspace_root: workspace.clone(),
            project_root: workspace,
        })
        .expect("targets");

        assert!(targets.iter().any(|target| {
            target.path == common_dir && target.mode == RecursiveMode::Recursive
        }));
        assert!(git_dir.starts_with(&common_dir));
    }
}
