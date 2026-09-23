use std::path::PathBuf;

use gpui::{Context, FocusHandle, Window};

use super::FarcasterApp;
use crate::{runtime::RuntimeCommand, sessions::archived_root_family_for_path};

pub(in crate::app) struct PendingDelete {
    pub(in crate::app) focus: FocusHandle,
    path: PathBuf,
    return_focus: Option<FocusHandle>,
}

impl FarcasterApp {
    pub(in crate::app) fn request_session_delete(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(_) = archived_root_family_for_path(&self.sessions.all, &path) else {
            self.sessions.error = Some("Only an archived root session can be deleted".to_owned());
            self.notify_session_rail(cx);
            return;
        };
        self.cover_native_workspace_surface(cx);
        let pending = PendingDelete {
            focus: cx.focus_handle(),
            path,
            return_focus: window.focused(cx),
        };
        pending.focus.focus(window, cx);
        self.sessions.pending_delete = Some(pending);
        cx.notify();
    }

    pub(in crate::app) fn stop_and_delete_pending_session(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.close_delete_confirmation(window, cx) else {
            return;
        };
        self.send(
            RuntimeCommand::StopAndDeleteSessionFamily { path: pending.path },
            cx,
        );
    }

    pub(in crate::app) fn close_delete_confirmation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PendingDelete> {
        let pending = self.sessions.pending_delete.take()?;
        self.restore_overlay_focus(pending.return_focus.clone(), &pending.focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
        Some(pending)
    }
}
