//! Generation-guarded application orchestration for the read-only Changes projection.

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::PathBuf,
    time::SystemTime,
};

use gpui::{AppContext as _, Context, FocusHandle, ScrollHandle, Window};

use super::PiApp;
use crate::{
    conversation::ToolPresentation,
    session_changes::{self, ChangeSet, FileChange, FileChangeKind, FullDiff},
    sessions::{descendant_sessions, root_session_for_path},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FullDiffMode {
    Split,
    Unified,
}

#[derive(Clone, Debug)]
pub(crate) enum DiffSurface {
    Loading(FileChange),
    Ready(FileChange, FullDiff),
    Preview(FileChange, FullDiff, String),
    Error(FileChange, String),
}

#[derive(Default)]
struct RefreshGate {
    root_identity: Option<String>,
    fingerprint: Option<u64>,
    generation: u64,
    in_flight: bool,
    pending: bool,
}

impl RefreshGate {
    fn select_root(&mut self, next: Option<String>) -> bool {
        if self.root_identity == next {
            return false;
        }
        self.root_identity = next;
        self.fingerprint = None;
        self.generation = self.generation.saturating_add(1);
        self.in_flight = false;
        self.pending = false;
        true
    }

    fn request(&mut self, fingerprint: u64) -> Option<u64> {
        if self.fingerprint == Some(fingerprint) {
            return None;
        }
        self.fingerprint = Some(fingerprint);
        if self.in_flight {
            self.pending = true;
            return None;
        }
        self.in_flight = true;
        self.generation = self.generation.saturating_add(1);
        Some(self.generation)
    }

    fn finish(&mut self, generation: u64) -> Option<bool> {
        if generation != self.generation {
            return None;
        }
        self.in_flight = false;
        let rerun = std::mem::take(&mut self.pending);
        if rerun {
            self.fingerprint = None;
        }
        Some(rerun)
    }
}

pub(crate) struct ChangesState {
    pub set: ChangeSet,
    refresh: RefreshGate,
    pub row_focus: HashMap<PathBuf, FocusHandle>,
    pub diff: Option<DiffSurface>,
    diff_generation: u64,
    pub diff_mode: FullDiffMode,
    pub diff_scroll: ScrollHandle,
    pub diff_focus: FocusHandle,
    pub return_focus: Option<FocusHandle>,
    pub pending_diff_setup: bool,
}

impl ChangesState {
    pub fn new(cx: &mut Context<PiApp>) -> Self {
        Self {
            set: ChangeSet::default(),
            refresh: RefreshGate::default(),
            row_focus: HashMap::new(),
            diff: None,
            diff_generation: 0,
            diff_mode: FullDiffMode::Split,
            diff_scroll: ScrollHandle::new(),
            diff_focus: cx.focus_handle(),
            return_focus: None,
            pending_diff_setup: false,
        }
    }
}

impl PiApp {
    pub(crate) fn request_changes_refresh(&mut self, cx: &mut Context<Self>) {
        let root = root_session_for_path(
            &self.all_sessions,
            self.snapshot.selected_session.as_deref(),
        );
        let identity = root.map(|root| root.path.to_string_lossy().into_owned());
        if self.changes.refresh.select_root(identity) {
            self.changes.set = ChangeSet::default();
            self.changes.row_focus.clear();
            self.changes.diff_generation = self.changes.diff_generation.saturating_add(1);
            self.changes.diff = None;
            self.changes.return_focus = None;
            self.changes.pending_diff_setup = false;
        }
        let Some(root) = root else {
            return;
        };
        let descendants = descendant_sessions(&self.all_sessions, &root.id);
        let mut ids = vec![root.id.clone()];
        ids.extend(
            descendants
                .into_iter()
                .map(|(session, _)| session.id.clone()),
        );
        let mut observed = Vec::<(PathBuf, SystemTime)>::new();
        for id in ids {
            if let Some(activity) = self.agent_activities.get(&id) {
                observed.extend(
                    activity
                        .changed_paths
                        .iter()
                        .map(|path| (path.path.clone(), path.observed_at)),
                );
            }
        }
        observed.sort_by(|left, right| left.0.cmp(&right.0));
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        root.id.hash(&mut hasher);
        for (path, time) in &observed {
            path.hash(&mut hasher);
            time.duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
                .hash(&mut hasher);
        }
        let fingerprint = hasher.finish();
        let Some(generation) = self.changes.refresh.request(fingerprint) else {
            return;
        };
        let project = root.project.clone();
        let task = cx.background_spawn(async move { session_changes::collect(&project, observed) });
        cx.spawn(async move |weak, cx| {
            let set = task.await;
            let _ = weak.update(cx, |this, cx| {
                let Some(rerun) = this.changes.refresh.finish(generation) else {
                    return;
                };
                this.changes
                    .row_focus
                    .retain(|path, _| set.files.iter().any(|file| &file.path == path));
                for file in &set.files {
                    this.changes
                        .row_focus
                        .entry(file.path.clone())
                        .or_insert_with(|| cx.focus_handle());
                }
                this.changes.set = set;
                this.notify_run_panel(cx);
                if rerun {
                    this.request_changes_refresh(cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn open_file_diff(
        &mut self,
        file: FileChange,
        opener: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        opener.focus(window, cx);
        self.changes.return_focus = Some(opener);
        self.changes.diff_generation = self.changes.diff_generation.saturating_add(1);
        let generation = self.changes.diff_generation;
        self.changes.diff = Some(DiffSurface::Loading(file.clone()));
        self.changes.pending_diff_setup = true;
        let root = self.changes.set.repo_root.clone();
        let task = cx.background_spawn(async move {
            root.ok_or_else(|| "Repository root is unavailable".to_owned())
                .and_then(|root| session_changes::load_full_diff(&root, &file))
                .map(|diff| (file, diff))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                if generation != this.changes.diff_generation {
                    return;
                }
                this.changes.diff = Some(match result {
                    Ok((file, diff)) => DiffSurface::Ready(file, diff),
                    Err(error) => {
                        let file = match this.changes.diff.take() {
                            Some(
                                DiffSurface::Loading(file)
                                | DiffSurface::Ready(file, _)
                                | DiffSurface::Preview(file, _, _)
                                | DiffSurface::Error(file, _),
                            ) => file,
                            None => return,
                        };
                        DiffSurface::Error(file, error)
                    }
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn open_tool_diff(
        &mut self,
        presentation: ToolPresentation,
        opener: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let presentation_path = match &presentation {
            ToolPresentation::Edit { path, .. } | ToolPresentation::Write { path, .. } => path,
        };
        let project = root_session_for_path(
            &self.all_sessions,
            self.snapshot.selected_session.as_deref(),
        )
        .map(|root| root.project.clone())
        .unwrap_or_else(|| self.project.clone());
        let project = crate::sessions::normalize_lexical(&project);
        let path = crate::sessions::normalize_lexical(&project.join(presentation_path));
        let loading_file = tool_file(&presentation, path.clone(), false);
        let focus = opener.unwrap_or_else(|| self.composer_focus.clone());
        focus.focus(window, cx);
        self.changes.return_focus = Some(focus);
        self.changes.diff_generation = self.changes.diff_generation.saturating_add(1);
        let generation = self.changes.diff_generation;
        self.changes.diff = Some(DiffSurface::Loading(loading_file));
        self.changes.pending_diff_setup = true;
        let task = cx
            .background_spawn(async move { load_tool_diff_surface(&project, path, presentation) });
        cx.spawn(async move |weak, cx| {
            let surface = task.await;
            let _ = weak.update(cx, |this, cx| {
                if generation != this.changes.diff_generation {
                    return;
                }
                this.changes.diff = Some(surface);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn close_file_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.changes.diff_generation = self.changes.diff_generation.saturating_add(1);
        self.changes.diff = None;
        self.changes.pending_diff_setup = false;
        self.changes
            .return_focus
            .take()
            .unwrap_or_else(|| self.composer_focus.clone())
            .focus(window, cx);
        cx.notify();
    }
}

fn tool_file(presentation: &ToolPresentation, path: PathBuf, exists: bool) -> FileChange {
    let kind = match presentation {
        ToolPresentation::Edit { .. } => FileChangeKind::Modified,
        ToolPresentation::Write { .. } => FileChangeKind::Added,
    };
    FileChange {
        path,
        old_path: None,
        kind,
        additions: None,
        deletions: None,
        observed_at: SystemTime::now(),
        exists,
    }
}

fn load_tool_diff_surface(
    project: &std::path::Path,
    path: PathBuf,
    presentation: ToolPresentation,
) -> DiffSurface {
    match session_changes::load_current_path_diff(project, &path) {
        Ok((file, diff)) => DiffSurface::Ready(file, diff),
        Err(error) => {
            let file = tool_file(&presentation, path.clone(), path.exists());
            let patch = match presentation {
                ToolPresentation::Edit { diff, .. } => diff,
                ToolPresentation::Write { content, .. } => {
                    Some(content.lines().map(|line| format!("+{line}\n")).collect())
                }
            };
            match patch {
                Some(patch) => DiffSurface::Preview(
                    file,
                    FullDiff {
                        path,
                        patch,
                        binary: false,
                    },
                    error,
                ),
                None => DiffSurface::Error(file, error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{DiffSurface, RefreshGate, load_tool_diff_surface};
    use crate::conversation::{EditDiffFormat, ToolPresentation};
    use std::{fs, path::Path, process::Command, sync::Arc};

    fn edit(path: &str, diff: Option<&str>) -> ToolPresentation {
        ToolPresentation::Edit {
            path: path.into(),
            diff: diff.map(str::to_owned),
            format: EditDiffFormat::Unnumbered,
            prepared: Arc::default(),
        }
    }

    fn git(repo: &Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .current_dir(repo)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn transcript_expand_loads_current_diff_before_catalog_is_available() {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-q"]);
        fs::write(repo.path().join("file.txt"), "before\n").unwrap();
        git(repo.path(), &["add", "file.txt"]);
        git(
            repo.path(),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "initial",
            ],
        );
        fs::write(repo.path().join("file.txt"), "current\n").unwrap();

        let surface = load_tool_diff_surface(
            repo.path(),
            repo.path().join("file.txt"),
            edit("file.txt", Some("-retained\n+preview\n")),
        );
        let DiffSurface::Ready(_, diff) = surface else {
            panic!("current repository diff should be complete");
        };
        assert!(diff.patch.contains("+current"));
        assert!(!diff.patch.contains("+preview"));

        let untracked = repo.path().join("untracked.txt");
        fs::write(&untracked, "from worktree\n").unwrap();
        let surface = load_tool_diff_surface(
            repo.path(),
            untracked,
            edit("untracked.txt", Some("+retained only\n")),
        );
        let DiffSurface::Ready(_, diff) = surface else {
            panic!("untracked repository diff should use the complete no-index path");
        };
        assert!(diff.patch.contains("+from worktree"));
        assert!(!diff.patch.contains("+retained only"));
    }

    #[test]
    fn non_git_transcript_expand_is_truthfully_a_preview() {
        let project = tempfile::tempdir().unwrap();
        let path = project.path().join("file.txt");
        fs::write(&path, "current\n").unwrap();
        let surface = load_tool_diff_surface(
            project.path(),
            path,
            edit("file.txt", Some("-old\n+retained\n")),
        );
        let DiffSurface::Preview(_, diff, reason) = surface else {
            panic!("retained inline data must be labelled as a preview");
        };
        assert!(diff.patch.contains("+retained"));
        assert!(reason.contains("not a Git repository"));
    }

    #[test]
    fn root_identity_invalidates_stale_work_and_revisiting_refreshes() {
        let mut gate = RefreshGate::default();
        assert!(gate.select_root(Some("root-a".into())));
        let stale = gate.request(7).expect("first request");
        assert!(gate.select_root(Some("root-b".into())));
        let current = gate.request(7).expect("same fingerprint on new root");
        assert_ne!(stale, current);
        assert_eq!(gate.finish(stale), None);
        assert_eq!(gate.finish(current), Some(false));

        assert!(gate.select_root(None));
        assert!(gate.select_root(Some("root-a".into())));
        assert!(gate.request(7).is_some());
    }
}
