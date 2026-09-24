use crate::agents::Backend;
use std::{collections::HashSet, path::PathBuf};

use gpui::{Context, FocusHandle, Window};

use super::FarcasterApp;
use crate::{agents, runtime::RuntimeCommand, sessions::SessionSummary};

pub(in crate::app) struct SessionImportDialog {
    pub(in crate::app) focus: FocusHandle,
    return_focus: Option<FocusHandle>,
    pub(in crate::app) harness: Option<Backend>,
    pub(in crate::app) profile_id: Option<String>,
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
        let preferred_profile = self
            .sessions
            .preferred_profile_id
            .as_deref()
            .and_then(|id| self.settings.harness_profiles.get(id).ok());
        let harness = preferred_profile
            .as_ref()
            .map(|profile| profile.backend)
            .or_else(|| {
                available
                    .into_iter()
                    .find(|harness| Some(*harness) == self.sessions.preferred_harness)
            });
        let dialog = SessionImportDialog {
            focus: cx.focus_handle(),
            return_focus: window.focused(cx),
            harness,
            profile_id: preferred_profile.map(|profile| profile.id),
            preview_generation: 0,
            candidates: Vec::new(),
            selected: HashSet::new(),
            loading: false,
            error: None,
        };
        dialog.focus.focus(window, cx);
        self.sessions.import = Some(dialog);
        self.preview_session_import(cx);
        cx.notify();
    }

    pub(in crate::app) fn close_session_import(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.sessions.import.take() else {
            return;
        };
        self.restore_overlay_focus(dialog.return_focus, &dialog.focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
    }

    pub(in crate::app) fn select_session_import_harness(
        &mut self,
        harness: Backend,
        cx: &mut Context<Self>,
    ) {
        {
            let Some(dialog) = self.sessions.import.as_mut() else {
                return;
            };
            if dialog.harness == Some(harness)
                && dialog.profile_id.is_none()
                && (dialog.loading || dialog.error.is_none())
            {
                return;
            }
            dialog.harness = Some(harness);
            dialog.profile_id = None;
        }
        self.preview_session_import(cx);
    }

    pub(in crate::app) fn select_session_import_profile(
        &mut self,
        harness: Backend,
        profile_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.sessions.import.as_mut() else {
            return;
        };
        if dialog.profile_id.as_deref() == Some(profile_id.as_str())
            && (dialog.loading || dialog.error.is_none())
        {
            return;
        }
        dialog.harness = Some(harness);
        dialog.profile_id = Some(profile_id);
        self.preview_session_import(cx);
    }

    pub(in crate::app) fn toggle_session_import_candidate(
        &mut self,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.sessions.import.as_mut() else {
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
        let Some(dialog) = self.sessions.import.as_mut() else {
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
        let sessions = self.sessions.import.as_ref().map(|dialog| {
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
        harness: Backend,
        sessions: Vec<SessionSummary>,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.sessions.import.as_mut() else {
            return;
        };
        if dialog.preview_generation != generation || dialog.harness != Some(harness) {
            return;
        }
        dialog.loading = false;
        dialog.error = None;
        dialog.selected.clear();
        dialog.candidates = sessions;
        cx.notify();
    }

    pub(in crate::app) fn apply_import_preview_failed(
        &mut self,
        generation: u64,
        harness: Backend,
        message: String,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.sessions.import.as_mut() else {
            return;
        };
        if dialog.preview_generation != generation || dialog.harness != Some(harness) {
            return;
        }
        dialog.loading = false;
        dialog.candidates.clear();
        dialog.selected.clear();
        dialog.error = Some(message);
        cx.notify();
    }

    fn preview_session_import(&mut self, cx: &mut Context<Self>) {
        self.sessions.import_generation = self.sessions.import_generation.saturating_add(1);
        let Some((harness, profile_id, generation)) = self.sessions.import.as_mut().map(|dialog| {
            dialog.preview_generation = self.sessions.import_generation;
            dialog.loading = dialog.harness.is_some();
            dialog.error = dialog
                .harness
                .is_none()
                .then(|| "Choose a backend to import sessions.".into());
            dialog.candidates.clear();
            dialog.selected.clear();
            (
                dialog.harness,
                dialog.profile_id.clone(),
                dialog.preview_generation,
            )
        }) else {
            return;
        };
        let Some(harness) = harness else {
            return;
        };
        self.send(
            RuntimeCommand::PreviewImport {
                harness,
                profile_id,
                generation,
            },
            cx,
        );
        cx.notify();
    }
}

pub(in crate::app) fn import_harnesses() -> Vec<Backend> {
    agents::backend_statuses()
        .into_iter()
        .filter(|backend| backend.available)
        .map(|backend| backend.id)
        .collect()
}
