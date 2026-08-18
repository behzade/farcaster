//! Application state for changes retained in Pi session records.

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::PathBuf,
    time::SystemTime,
};

use gpui::{AppContext as _, Context, FocusHandle, ScrollHandle, Window, point, px};

use super::PiApp;
use crate::{
    agent_activity::{FileMutation, FileMutationKind},
    conversation::{EditDiffFormat, ToolPresentation},
    session_changes::{self, ChangeSet, FileChange, FullDiff},
    sessions::{descendant_sessions, root_session_for_path},
    syntax_highlight::HighlightedDiff,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FullDiffMode {
    Split,
    Unified,
}

#[derive(Clone, Debug)]
pub(crate) enum DiffSurface {
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
    pub diff_syntax: Option<HighlightedDiff>,
    diff_generation: u64,
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
            diff_syntax: None,
            diff_generation: 0,
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
            self.changes.diff_syntax = None;
            self.changes.return_focus = None;
            self.changes.pending_diff_setup = false;
        }
        let Some(root) = root else {
            return;
        };
        let project = crate::sessions::normalize_lexical(&root.project);
        let descendants = descendant_sessions(&self.all_sessions, &root.id);
        let mut ids = vec![root.id.clone()];
        ids.extend(
            descendants
                .into_iter()
                .map(|(session, _)| session.id.clone()),
        );
        let mut mutations = Vec::<FileMutation>::new();
        let mut incomplete = false;
        for id in ids {
            if let Some(activity) = self.agent_activities.get(&id) {
                incomplete |= activity.limited;
                mutations.extend(activity.file_mutations.iter().cloned());
            }
        }
        mutations.retain(|mutation| is_project_change(&mutation.path, &project));
        mutations.sort_by_key(|mutation| mutation.observed_at);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        root.id.hash(&mut hasher);
        incomplete.hash(&mut hasher);
        for mutation in &mutations {
            mutation.hash(&mut hasher);
        }
        let fingerprint = hasher.finish();
        let Some(generation) = self.changes.refresh.request(fingerprint) else {
            return;
        };
        let task = cx.background_spawn(async move {
            let mut set = session_changes::collect(mutations);
            set.incomplete = incomplete;
            set
        });
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
        self.changes.diff_scroll.set_offset(point(px(0.0), px(0.0)));
        self.changes.diff_syntax = Some(HighlightedDiff::new(
            &file.path.to_string_lossy(),
            &file.diff.patch,
        ));
        self.changes.diff = Some(DiffSurface::Ready(file.clone(), file.diff.clone()));
        self.changes.pending_diff_setup = true;
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
        let focus = opener.unwrap_or_else(|| self.composer_focus.clone());
        focus.focus(window, cx);
        self.changes.return_focus = Some(focus);
        self.changes.diff_generation = self.changes.diff_generation.saturating_add(1);
        self.changes.diff_scroll.set_offset(point(px(0.0), px(0.0)));
        let surface = load_tool_diff_surface(path, presentation);
        self.changes.diff_syntax = surface_diff(&surface)
            .map(|(path, patch)| HighlightedDiff::new(&path.to_string_lossy(), patch));
        self.changes.diff = Some(surface);
        self.changes.pending_diff_setup = true;
        cx.notify();
    }

    pub(crate) fn close_file_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.changes.diff_generation = self.changes.diff_generation.saturating_add(1);
        self.changes.diff = None;
        self.changes.diff_syntax = None;
        self.changes.pending_diff_setup = false;
        self.changes
            .return_focus
            .take()
            .unwrap_or_else(|| self.composer_focus.clone())
            .focus(window, cx);
        cx.notify();
    }
}

fn surface_diff(surface: &DiffSurface) -> Option<(&std::path::Path, &str)> {
    match surface {
        DiffSurface::Ready(file, diff) | DiffSurface::Preview(file, diff, _) => {
            Some((&file.path, &diff.patch))
        }
        DiffSurface::Error(_, _) => None,
    }
}

fn is_project_change(path: &std::path::Path, project: &std::path::Path) -> bool {
    path.starts_with(project) && path != project
}

fn load_tool_diff_surface(path: PathBuf, presentation: ToolPresentation) -> DiffSurface {
    let kind = match presentation {
        ToolPresentation::Edit { diff, format, .. } => FileMutationKind::Edit {
            patch: diff.unwrap_or_default(),
            complete: format == EditDiffFormat::Numbered,
        },
        ToolPresentation::Write { content, .. } => FileMutationKind::Write { content },
    };
    let mut set = session_changes::collect([FileMutation {
        path,
        observed_at: SystemTime::now(),
        kind,
    }]);
    let Some(file) = set.files.pop() else {
        unreachable!("one tool mutation produces one file change");
    };
    let diff = file.diff.clone();
    if diff.patch.contains("Recorded edit has no retained diff.") {
        DiffSurface::Error(file, "Pi did not retain a diff for this edit".into())
    } else if diff.partial {
        DiffSurface::Preview(
            file,
            diff,
            "Pi retained the edit arguments but no completed tool-result patch".into(),
        )
    } else {
        DiffSurface::Ready(file, diff)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{DiffSurface, RefreshGate, load_tool_diff_surface};
    use crate::conversation::{EditDiffFormat, ToolPresentation};
    use std::{path::PathBuf, sync::Arc};

    fn edit(path: &str, diff: Option<&str>) -> ToolPresentation {
        ToolPresentation::Edit {
            path: path.into(),
            diff: diff.map(str::to_owned),
            format: EditDiffFormat::Unnumbered,
            prepared: Arc::default(),
        }
    }

    #[test]
    fn transcript_expand_uses_the_recorded_tool_change() {
        let surface = load_tool_diff_surface(
            PathBuf::from("/project/file.txt"),
            ToolPresentation::Edit {
                path: "file.txt".into(),
                diff: Some("@@\n-session value\n+recorded value\n".into()),
                format: EditDiffFormat::Numbered,
                prepared: Arc::default(),
            },
        );
        let DiffSurface::Ready(_, diff) = surface else {
            panic!("completed tool result should be complete");
        };
        assert!(diff.patch.contains("+recorded value"));
        assert!(!diff.patch.contains("HEAD"));
    }

    #[test]
    fn argument_only_edit_is_truthfully_a_preview() {
        let surface = load_tool_diff_surface(
            PathBuf::from("/project/file.txt"),
            edit("file.txt", Some("-old\n+retained\n")),
        );
        let DiffSurface::Preview(_, diff, reason) = surface else {
            panic!("retained inline data must be labelled as a preview");
        };
        assert!(diff.patch.contains("+retained"));
        assert!(reason.contains("no completed tool-result patch"));
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
