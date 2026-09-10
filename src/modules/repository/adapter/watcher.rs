use std::{
    fs,
    path::{Path, PathBuf},
};

use async_channel::Receiver;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

use super::super::{RepositoryKind, RepositoryLocation};

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum RepositoryWatchEvent {
    Changed,
    Failed(String),
}

pub(crate) struct RepositoryWatcher {
    watcher: Option<RecommendedWatcher>,
}

impl Drop for RepositoryWatcher {
    fn drop(&mut self) {
        let Some(watcher) = self.watcher.take() else {
            return;
        };
        let _ = std::thread::Builder::new()
            .name("repository-watcher-drop".into())
            .spawn(move || drop(watcher));
    }
}

impl RepositoryWatcher {
    pub(crate) fn start(
        location: &RepositoryLocation,
    ) -> Result<(Self, Receiver<RepositoryWatchEvent>), String> {
        let mut targets = watch_targets(location)?;
        match location.kind {
            RepositoryKind::Git => Self::start_targets(targets, repository_event),
            RepositoryKind::Jujutsu => {
                let metadata = JujutsuMetadata::resolve(location)?;
                add_existing_target(
                    &mut targets,
                    metadata.repo.join("op_heads"),
                    RecursiveMode::Recursive,
                )?;
                for target in &metadata.git {
                    add_target(&mut targets, target.path.clone(), target.mode);
                }
                Self::start_targets(targets, move |event| metadata.classify(event))
            }
        }
    }

    pub(crate) fn start_discovery(
        project: &Path,
    ) -> Result<(Self, Receiver<RepositoryWatchEvent>), String> {
        Self::start_targets(discovery_targets(project)?, discovery_event)
    }

    fn start_targets(
        targets: Vec<WatchTarget>,
        classify: impl Fn(notify::Result<Event>) -> Option<RepositoryWatchEvent> + Send + 'static,
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
        Ok((
            Self {
                watcher: Some(watcher),
            },
            receiver,
        ))
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

struct JujutsuMetadata {
    repo: PathBuf,
    git: Vec<WatchTarget>,
}

impl JujutsuMetadata {
    fn resolve(location: &RepositoryLocation) -> Result<Self, String> {
        let directory = location.workspace_root.join(".jj");
        let marker = directory.join("repo");
        let repo = if marker.is_file() {
            let value = fs::read_to_string(&marker)
                .map_err(|error| format!("read {}: {error}", marker.display()))?;
            resolve_relative(&directory, Path::new(value.trim()))?
        } else {
            resolve_relative(&directory, Path::new("repo"))?
        };
        let mut git = Vec::new();
        if location.workspace_root.join(".git").exists() {
            add_git_targets(&mut git, &location.workspace_root)?;
        }
        Ok(Self { repo, git })
    }

    fn classify(&self, result: notify::Result<Event>) -> Option<RepositoryWatchEvent> {
        match result {
            Ok(event) if !event.paths.iter().any(|path| self.changed(path)) => None,
            result => repository_event(result),
        }
    }

    fn changed(&self, path: &Path) -> bool {
        if let Ok(relative) = path.strip_prefix(&self.repo) {
            // Snapshot reads touch locks and tree state. Only published operations
            // should cause another refresh.
            return relative.starts_with("op_heads/heads")
                && path.file_name().is_some_and(|name| {
                    let name = name.to_string_lossy();
                    !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_hexdigit())
                });
        }
        for target in &self.git {
            if let Ok(relative) = path.strip_prefix(&target.path) {
                return (relative == Path::new("HEAD")
                    || relative == Path::new("packed-refs")
                    || relative.starts_with("refs")
                    || (relative.starts_with("worktrees")
                        && relative.file_name().is_some_and(|name| name == "HEAD")))
                    && relative
                        .extension()
                        .is_none_or(|extension| extension != "lock");
            }
        }
        !path.components().any(|component| {
            let component = component.as_os_str();
            component == ".jj" || component == ".git"
        })
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
    // Only the project and its parents can gain a marker for this project.
    Ok(project
        .ancestors()
        .map(|path| WatchTarget {
            path: path.to_path_buf(),
            mode: RecursiveMode::NonRecursive,
        })
        .collect())
}

fn watch_targets(location: &RepositoryLocation) -> Result<Vec<WatchTarget>, String> {
    let mut targets = Vec::new();
    add_target(
        &mut targets,
        location.project_root.clone(),
        RecursiveMode::Recursive,
    );
    if location.kind == RepositoryKind::Git {
        add_git_targets(&mut targets, &location.workspace_root)?;
    }
    Ok(targets)
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
#[path = "watcher_tests.rs"]
mod tests;
