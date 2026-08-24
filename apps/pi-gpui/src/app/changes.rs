//! Application state for changes retained in Pi session records.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::Arc,
    time::SystemTime,
};

use gpui::{AppContext as _, Context, FocusHandle, ScrollHandle, Window, px};

use super::PiApp;
use crate::{
    agent_activity::{FileMutation, FileMutationKind},
    conversation::{EditDiffFormat, ToolPresentation},
    repository::{DiffResult as RepositoryDiff, DiffTarget, RepositoryBackend},
    session_changes::{self, ChangeSet, FileChange, FullDiff},
    sessions::{descendant_sessions, root_session_for_path},
    syntax_highlight::{DiffHighlightMode, HighlightedDiff},
};

const DIFF_HIGHLIGHT_CACHE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum FullDiffMode {
    Split,
    Unified,
}

#[derive(Clone, Debug)]
pub(crate) enum RepositoryDiffState {
    Loading,
    Ready(Box<RepositoryDiff>),
    Error(String),
}

#[derive(Clone, Debug)]
pub(crate) enum DiffSurface {
    Ready(FileChange, FullDiff),
    Preview(FileChange, FullDiff, String),
    Error(FileChange, String),
    Repository {
        target: Box<DiffTarget>,
        state: RepositoryDiffState,
    },
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct DiffHighlightKey {
    path: String,
    patch_hash: u64,
    mode: FullDiffMode,
}

#[derive(Default)]
struct DiffHighlightCache {
    entries: HashMap<DiffHighlightKey, (Arc<HighlightedDiff>, usize)>,
    order: VecDeque<DiffHighlightKey>,
    bytes: usize,
}

impl DiffHighlightCache {
    fn get(&mut self, key: &DiffHighlightKey) -> Option<Arc<HighlightedDiff>> {
        let value = self.entries.get(key)?.0.clone();
        self.order.retain(|candidate| candidate != key);
        self.order.push_back(key.clone());
        Some(value)
    }

    fn insert(&mut self, key: DiffHighlightKey, value: Arc<HighlightedDiff>, bytes: usize) {
        if bytes > DIFF_HIGHLIGHT_CACHE_BYTES {
            return;
        }
        if let Some((_, previous_bytes)) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(previous_bytes);
            self.order.retain(|candidate| candidate != &key);
        }
        while self.bytes.saturating_add(bytes) > DIFF_HIGHLIGHT_CACHE_BYTES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some((_, removed_bytes)) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(removed_bytes);
            }
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.order.push_back(key.clone());
        self.entries.insert(key, (value, bytes));
    }
}

pub(crate) struct ChangesState {
    pub set: ChangeSet,
    refresh: RefreshGate,
    pub row_focus: HashMap<PathBuf, FocusHandle>,
    pub diff: Option<DiffSurface>,
    pub diff_syntax: Option<Arc<HighlightedDiff>>,
    diff_generation: u64,
    diff_open_timing: Option<crate::performance::Timing>,
    diff_highlights: DiffHighlightCache,
    diff_highlights_in_flight: HashSet<DiffHighlightKey>,
    diff_highlight_requested: Option<DiffHighlightKey>,
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
            diff_open_timing: None,
            diff_highlights: DiffHighlightCache::default(),
            diff_highlights_in_flight: HashSet::new(),
            diff_highlight_requested: None,
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
            cancel_timing(&mut self.changes.diff_open_timing);
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
        cancel_timing(&mut self.changes.diff_open_timing);
        self.changes.diff_open_timing = Some(crate::performance::Timing::new_always(
            "diff.open_to_highlight_ready",
        ));
        self.changes.diff_scroll = ScrollHandle::new();
        let surface = DiffSurface::Ready(file.clone(), file.diff.clone());
        self.changes.diff_syntax = None;
        self.changes.diff = Some(surface);
        self.changes.pending_diff_setup = true;
        self.ensure_diff_highlight(full_diff_mode(window), cx);
        cx.notify();
    }

    pub(crate) fn open_repository_diff(
        &mut self,
        backend: RepositoryBackend,
        target: DiffTarget,
        opener: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        opener.focus(window, cx);
        self.changes.return_focus = Some(opener);
        self.changes.diff_generation = self.changes.diff_generation.saturating_add(1);
        let generation = self.changes.diff_generation;
        cancel_timing(&mut self.changes.diff_open_timing);
        self.changes.diff_scroll = ScrollHandle::new();
        self.changes.diff_syntax = None;
        self.changes.diff = Some(DiffSurface::Repository {
            target: Box::new(target.clone()),
            state: RepositoryDiffState::Loading,
        });
        self.changes.pending_diff_setup = true;
        cx.notify();

        let task_target = target.clone();
        let task = cx.background_spawn(async move { backend.load_diff(task_target) });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                if this.changes.diff_generation != generation {
                    return;
                }
                let Some(DiffSurface::Repository {
                    target: current,
                    state,
                }) = this.changes.diff.as_mut()
                else {
                    return;
                };
                if current.key != target.key {
                    return;
                }
                match result {
                    Ok(diff) => {
                        this.changes.diff_open_timing = Some(
                            crate::performance::Timing::new_always("diff.open_to_highlight_ready"),
                        );
                        *state = RepositoryDiffState::Ready(Box::new(diff));
                    }
                    Err(error) => *state = RepositoryDiffState::Error(error.to_string()),
                }
                this.changes.diff_syntax = None;
                cx.notify();
            });
        })
        .detach();
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
        cancel_timing(&mut self.changes.diff_open_timing);
        self.changes.diff_open_timing = Some(crate::performance::Timing::new_always(
            "diff.open_to_highlight_ready",
        ));
        self.changes.diff_scroll = ScrollHandle::new();
        let surface = {
            let _operation = crate::performance::OperationTiming::new(
                crate::performance::OperationKind::FullDiffPrepare,
                0,
            );
            let _timing = crate::performance::Timing::new_always("diff.reconstruct_tool_surface");
            load_tool_diff_surface(path, presentation)
        };
        self.changes.diff_syntax = None;
        self.changes.diff = Some(surface);
        self.changes.pending_diff_setup = true;
        self.ensure_diff_highlight(full_diff_mode(window), cx);
        cx.notify();
    }

    pub(super) fn ensure_diff_highlight(&mut self, mode: FullDiffMode, cx: &mut Context<Self>) {
        if self
            .changes
            .diff_syntax
            .as_ref()
            .is_some_and(|syntax| syntax.mode() == mode.highlight_mode())
        {
            return;
        }
        let Some(surface) = self.changes.diff.as_ref() else {
            return;
        };
        let Some((path, patch)) = surface_diff(surface) else {
            drop(self.changes.diff_open_timing.take());
            return;
        };
        let (path, patch, key) = {
            let _operation = crate::performance::OperationTiming::new(
                crate::performance::OperationKind::FullDiffPrepare,
                patch.len(),
            );
            let _timing = crate::performance::Timing::new_always("diff.prepare_highlight_input");
            let path = path.to_string_lossy().into_owned();
            let patch = patch.to_owned();
            let key = diff_highlight_key(&path, &patch, mode);
            (path, patch, key)
        };
        self.changes.diff_highlight_requested = Some(key.clone());
        let cached = {
            let _timing = crate::performance::Timing::new_always("diff.highlight_cache_lookup");
            self.changes.diff_highlights.get(&key)
        };
        if let Some(syntax) = cached {
            self.changes.diff_syntax = Some(syntax);
            drop(self.changes.diff_open_timing.take());
            return;
        }
        if !self.changes.diff_highlights_in_flight.insert(key.clone()) {
            return;
        }
        let generation = self.changes.diff_generation;
        let highlight_mode = mode.highlight_mode();
        let bytes = patch
            .len()
            .saturating_mul(if mode == FullDiffMode::Split { 2 } else { 1 });
        let task = cx.background_spawn(async move {
            let _timing = crate::performance::Timing::new_always("diff.highlight_compute");
            Arc::new(HighlightedDiff::new(&path, &patch, highlight_mode))
        });
        cx.spawn(async move |weak, cx| {
            let syntax = task.await;
            let _ = weak.update(cx, |this, cx| {
                let _timing = crate::performance::Timing::new_always("diff.publish_highlight");
                this.changes.diff_highlights_in_flight.remove(&key);
                this.changes
                    .diff_highlights
                    .insert(key.clone(), syntax.clone(), bytes);
                if this.changes.diff_generation == generation
                    && this.changes.diff_highlight_requested.as_ref() == Some(&key)
                    && this.changes.diff.is_some()
                {
                    this.changes.diff_syntax = Some(syntax);
                    drop(this.changes.diff_open_timing.take());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn invalidate_repository_diff(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.changes.diff, Some(DiffSurface::Repository { .. })) {
            return;
        }
        self.changes.diff_generation = self.changes.diff_generation.saturating_add(1);
        cancel_timing(&mut self.changes.diff_open_timing);
        self.changes.diff = None;
        self.changes.diff_syntax = None;
        self.changes.diff_highlight_requested = None;
        self.changes.pending_diff_setup = false;
        self.changes.return_focus = None;
        cx.notify();
    }

    pub(crate) fn close_file_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.changes.diff_generation = self.changes.diff_generation.saturating_add(1);
        cancel_timing(&mut self.changes.diff_open_timing);
        self.changes.diff = None;
        self.changes.diff_syntax = None;
        self.changes.diff_highlight_requested = None;
        self.changes.pending_diff_setup = false;
        self.changes
            .return_focus
            .take()
            .unwrap_or_else(|| self.composer_focus.clone())
            .focus(window, cx);
        cx.notify();
    }
}

impl FullDiffMode {
    fn highlight_mode(self) -> DiffHighlightMode {
        match self {
            Self::Split => DiffHighlightMode::Split,
            Self::Unified => DiffHighlightMode::Unified,
        }
    }
}

fn full_diff_mode(window: &Window) -> FullDiffMode {
    if crate::layout::shows_split_diff(window.viewport_size().width - px(64.0)) {
        FullDiffMode::Split
    } else {
        FullDiffMode::Unified
    }
}

fn diff_highlight_key(path: &str, patch: &str, mode: FullDiffMode) -> DiffHighlightKey {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    patch.hash(&mut hasher);
    DiffHighlightKey {
        path: path.to_owned(),
        patch_hash: hasher.finish(),
        mode,
    }
}

fn cancel_timing(timing: &mut Option<crate::performance::Timing>) {
    if let Some(timing) = timing.take() {
        timing.cancel();
    }
}

fn surface_diff(surface: &DiffSurface) -> Option<(PathBuf, &str)> {
    match surface {
        DiffSurface::Ready(file, diff) | DiffSurface::Preview(file, diff, _) => {
            Some((file.path.clone(), &diff.patch))
        }
        DiffSurface::Repository {
            state: RepositoryDiffState::Ready(diff),
            ..
        } => Some((diff.target.absolute_path(), &diff.patch)),
        DiffSurface::Error(_, _)
        | DiffSurface::Repository {
            state: RepositoryDiffState::Loading | RepositoryDiffState::Error(_),
            ..
        } => None,
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

    use super::{
        DIFF_HIGHLIGHT_CACHE_BYTES, DiffHighlightCache, DiffSurface, FullDiffMode, RefreshGate,
        diff_highlight_key, load_tool_diff_surface,
    };
    use crate::{
        conversation::{EditDiffFormat, ToolPresentation},
        syntax_highlight::HighlightedDiff,
    };
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
    fn diff_highlight_cache_is_mode_specific_and_bounded() {
        let mut cache = DiffHighlightCache::default();
        let unified = diff_highlight_key("file.rs", "patch", FullDiffMode::Unified);
        let split = diff_highlight_key("file.rs", "patch", FullDiffMode::Split);
        cache.insert(
            unified.clone(),
            Arc::new(HighlightedDiff::new(
                "file.txt",
                "patch",
                crate::syntax_highlight::DiffHighlightMode::Unified,
            )),
            DIFF_HIGHLIGHT_CACHE_BYTES / 2,
        );
        cache.insert(
            split.clone(),
            Arc::new(HighlightedDiff::new(
                "file.txt",
                "patch",
                crate::syntax_highlight::DiffHighlightMode::Split,
            )),
            DIFF_HIGHLIGHT_CACHE_BYTES / 2 + 1,
        );

        assert!(cache.get(&unified).is_none());
        assert!(cache.get(&split).is_some());
        assert!(cache.bytes <= DIFF_HIGHLIGHT_CACHE_BYTES);
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
