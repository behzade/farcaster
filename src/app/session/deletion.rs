use std::{collections::HashSet, path::PathBuf};

use gpui::{Context, FocusHandle, Window};

use super::FarcasterApp;
use crate::{runtime::RuntimeCommand, sessions::archived_root_family_for_path};

pub(in crate::app) struct PendingDelete {
    pub(in crate::app) focus: FocusHandle,
    path: PathBuf,
    family_paths: HashSet<PathBuf>,
    return_focus: Option<FocusHandle>,
}

impl FarcasterApp {
    pub(in crate::app) fn request_session_delete(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(family) = archived_root_family_for_path(&self.all_sessions, &path) else {
            self.sessions_error = Some("Only an archived root session can be deleted".to_owned());
            self.notify_session_rail(cx);
            return;
        };
        if self.session_family_has_active_work(&path) {
            self.sessions_error =
                Some("Wait for the session family to finish before deleting it".to_owned());
            self.notify_session_rail(cx);
            return;
        }
        let family_paths = family
            .into_iter()
            .map(|session| session.path.clone())
            .collect();
        self.cover_native_workspace_surface(cx);
        let pending = PendingDelete {
            focus: cx.focus_handle(),
            path,
            family_paths,
            return_focus: window.focused(cx),
        };
        pending.focus.focus(window, cx);
        self.pending_delete = Some(pending);
        cx.notify();
    }

    pub(in crate::app) fn delete_pending_session(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.close_delete_confirmation(window, cx) else {
            return;
        };
        self.sessions
            .retain(|session| !pending.family_paths.contains(&session.path));
        if !self.sessions.iter().any(|session| session.archived) {
            self.archived_sessions_expanded = false;
        }
        self.notify_session_rail(cx);
        self.send(
            RuntimeCommand::DeleteSessionFamily { path: pending.path },
            cx,
        );
    }

    pub(in crate::app) fn close_delete_confirmation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PendingDelete> {
        let pending = self.pending_delete.take()?;
        self.restore_overlay_focus(pending.return_focus.clone(), &pending.focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
        Some(pending)
    }
}
