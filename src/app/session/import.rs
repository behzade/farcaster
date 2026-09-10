use std::{collections::HashSet, path::PathBuf};

use gpui::{Context, FocusHandle, Window};

use super::FarcasterApp;
use crate::{agents, runtime::RuntimeCommand, sessions::SessionSummary};

pub(in crate::app) struct SessionImportDialog {
    pub(in crate::app) focus: FocusHandle,
    return_focus: Option<FocusHandle>,
    pub(in crate::app) harness: String,
    preview_generation: u64,
    pub(in crate::app) candidates: Vec<SessionSummary>,
    pub(in crate::app) selected: HashSet<PathBuf>,
    pub(in crate::app) loading: bool,
    pub(in crate::app) error: Option<String>,
}

impl FarcasterApp {
    pub(in crate::app) fn open_session_import(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cover_native_workspace_surface(cx);
        let available = import_harnesses();
        let harness = available
            .into_iter()
            .find(|harness| harness == &self.preferred_harness)
            .unwrap_or_default();
        let dialog = SessionImportDialog {
            focus: cx.focus_handle(),
            return_focus: window.focused(cx),
            harness,
            preview_generation: 0,
            candidates: Vec::new(),
            selected: HashSet::new(),
            loading: false,
            error: None,
        };
        dialog.focus.focus(window, cx);
        self.session_import = Some(dialog);
        self.preview_session_import(cx);
        cx.notify();
    }

    pub(in crate::app) fn close_session_import(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.session_import.take() else {
            return;
        };
        self.restore_overlay_focus(dialog.return_focus, &dialog.focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
    }

    pub(in crate::app) fn select_session_import_harness(
        &mut self,
        harness: String,
        cx: &mut Context<Self>,
    ) {
        {
            let Some(dialog) = self.session_import.as_mut() else {
                return;
            };
            if dialog.harness == harness && (dialog.loading || dialog.error.is_none()) {
                return;
            }
            dialog.harness = harness;
        }
        self.preview_session_import(cx);
    }

    pub(in crate::app) fn toggle_session_import_candidate(
        &mut self,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.session_import.as_mut() else {
            return;
        };
        if !dialog.selected.remove(&path) {
            dialog.selected.insert(path);
        }
        cx.notify();
    }

    pub(in crate::app) fn set_session_import_selection(
        &mut self,
        selected: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.session_import.as_mut() else {
            return;
        };
        dialog.selected = if selected {
            dialog
                .candidates
                .iter()
                .map(|session| session.path.clone())
                .collect()
        } else {
            HashSet::new()
        };
        cx.notify();
    }

    pub(in crate::app) fn confirm_session_import(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sessions = self.session_import.as_ref().map(|dialog| {
            dialog
                .candidates
                .iter()
                .filter(|session| dialog.selected.contains(&session.path))
                .cloned()
                .collect::<Vec<_>>()
        });
        let Some(sessions) = sessions.filter(|sessions| !sessions.is_empty()) else {
            return;
        };
        self.send(RuntimeCommand::CommitImport { sessions }, cx);
        self.close_session_import(window, cx);
    }

    pub(in crate::app) fn apply_import_preview(
        &mut self,
        generation: u64,
        harness: String,
        sessions: Vec<SessionSummary>,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.session_import.as_mut() else {
            return;
        };
        if dialog.preview_generation != generation || dialog.harness != harness {
            return;
        }
        dialog.loading = false;
        dialog.error = None;
        dialog.selected = sessions
            .iter()
            .map(|session| session.path.clone())
            .collect();
        dialog.candidates = sessions;
        cx.notify();
    }

    pub(in crate::app) fn apply_import_preview_failed(
        &mut self,
        generation: u64,
        harness: String,
        message: String,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.session_import.as_mut() else {
            return;
        };
        if dialog.preview_generation != generation || dialog.harness != harness {
            return;
        }
        dialog.loading = false;
        dialog.candidates.clear();
        dialog.selected.clear();
        dialog.error = Some(message);
        cx.notify();
    }

    fn preview_session_import(&mut self, cx: &mut Context<Self>) {
        self.session_import_generation = self.session_import_generation.saturating_add(1);
        let Some((harness, generation)) = self.session_import.as_mut().map(|dialog| {
            dialog.preview_generation = self.session_import_generation;
            dialog.loading = !dialog.harness.is_empty();
            dialog.error = dialog
                .harness
                .is_empty()
                .then(|| "Choose a backend to import sessions.".into());
            dialog.candidates.clear();
            dialog.selected.clear();
            (dialog.harness.clone(), dialog.preview_generation)
        }) else {
            return;
        };
        if harness.is_empty() {
            return;
        }
        self.send(
            RuntimeCommand::PreviewImport {
                harness,
                generation,
            },
            cx,
        );
        cx.notify();
    }
}

pub(in crate::app) fn import_harnesses() -> Vec<String> {
    agents::backend_statuses()
        .into_iter()
        .filter(|backend| backend.available)
        .map(|backend| backend.id)
        .collect()
}
