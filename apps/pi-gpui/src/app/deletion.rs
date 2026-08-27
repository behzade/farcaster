//! Session deletion confirmation flow.

use std::path::PathBuf;

use gpui::{Context, FocusHandle, Window};

use super::PiApp;
use crate::{runtime::RuntimeCommand, sessions::session_family_for_path};

pub(super) struct PendingDelete {
    pub(super) path: PathBuf,
    return_focus: Option<FocusHandle>,
}

impl PiApp {
    pub(super) fn request_session_delete(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if session_family_for_path(&self.all_sessions, &path)
            .is_some_and(|family| family.into_iter().any(|session| session.is_running))
        {
            self.sessions_error =
                Some("Wait for the session family to finish before deleting it".to_owned());
            self.notify_session_rail(cx);
            return;
        }
        self.cover_native_workspace_surface(cx);
        self.pending_delete = Some(PendingDelete {
            path,
            return_focus: window.focused(cx),
        });
        self.sheet_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn delete_pending_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.close_delete_confirmation(window, cx) else {
            return;
        };
        self.send(RuntimeCommand::DeleteSessionFamily { path });
    }

    pub(super) fn close_delete_confirmation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PathBuf> {
        let pending = self.pending_delete.take()?;
        pending
            .return_focus
            .unwrap_or_else(|| self.composer_focus.clone())
            .focus(window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
        Some(pending.path)
    }
}
