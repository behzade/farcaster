use std::path::{Path, PathBuf};

use gpui::{Context, FocusHandle, Window};

use super::FarcasterApp;
use crate::{
    app::composer::sessions::session_target,
    runtime::RuntimeCommand,
    sessions::{SessionSummary, root_session_for_path, session_family_for_path},
};

pub(in crate::app) struct PendingArchive {
    pub(in crate::app) focus: FocusHandle,
    path: PathBuf,
    return_focus: Option<FocusHandle>,
    next_app_session_id: Option<i64>,
}

impl FarcasterApp {
    pub(in crate::app) fn request_session_archive(
        &mut self,
        path: PathBuf,
        archive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let has_live_work = |path: &Path| {
            super::activity::session_has_live_work(path, &self.run_statuses, &self.snapshot)
                || self.pending_submissions.contains_key(&session_target(path))
        };
        let active = session_family_has_active_work(&self.all_sessions, &path, has_live_work);
        if !archive || !active {
            self.set_session_archived(path, archive, cx);
            return;
        }

        self.cover_native_workspace_surface(cx);
        let pending = PendingArchive {
            focus: cx.focus_handle(),
            path,
            return_focus: window.focused(cx),
            next_app_session_id: None,
        };
        pending.focus.focus(window, cx);
        self.pending_archive = Some(pending);
        cx.notify();
    }

    pub(in crate::app) fn request_session_archive_and_advance(
        &mut self,
        path: PathBuf,
        next_app_session_id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_session_archive(path, true, window, cx);
        if let Some(pending) = self.pending_archive.as_mut() {
            pending.next_app_session_id = next_app_session_id;
        } else if let Some(id) = next_app_session_id {
            self.select_visible_app_session(id, window, cx);
        }
    }

    pub(in crate::app) fn stop_and_archive_pending_session(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, next_app_session_id)) = self.close_archive_confirmation(window, cx) else {
            return;
        };
        self.send(RuntimeCommand::StopSessionFamily { path: path.clone() }, cx);
        self.set_session_archived(path, true, cx);
        if let Some(id) = next_app_session_id {
            self.select_visible_app_session(id, window, cx);
        }
    }

    pub(in crate::app) fn close_archive_confirmation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(PathBuf, Option<i64>)> {
        let pending = self.pending_archive.take()?;
        self.restore_overlay_focus(pending.return_focus.clone(), &pending.focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
        Some((pending.path, pending.next_app_session_id))
    }
}

fn session_family_has_active_work(
    sessions: &[SessionSummary],
    path: &Path,
    has_live_work: impl Fn(&Path) -> bool,
) -> bool {
    has_live_work(path)
        || session_family_for_path(sessions, path).is_some_and(|family| {
            family
                .into_iter()
                .any(|session| session.is_running || has_live_work(&session.path))
        })
}

pub(in crate::app) fn session_event_affects_archived_rail(
    sessions: &[SessionSummary],
    target: &str,
    session_path: Option<&Path>,
) -> bool {
    let session = session_path
        .and_then(|path| sessions.iter().find(|session| session.path == path))
        .or_else(|| {
            sessions
                .iter()
                .find(|session| session_target(&session.path) == target)
        });
    session
        .and_then(|session| root_session_for_path(sessions, Some(&session.path)))
        .is_some_and(|root| root.archived)
}

#[cfg(test)]
#[path = "archive_tests.rs"]
mod tests;
