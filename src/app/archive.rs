use std::path::{Path, PathBuf};

use gpui::{Context, FocusHandle, Window};

use super::FarcasterApp;
use crate::{
    composer_sessions::session_target,
    runtime::RuntimeCommand,
    sessions::{SessionSummary, root_session_for_path, session_family_for_path},
};

pub(super) struct PendingArchive {
    path: PathBuf,
    return_focus: Option<FocusHandle>,
}

impl FarcasterApp {
    pub(super) fn request_session_archive(
        &mut self,
        path: PathBuf,
        archive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !archive || !session_family_has_active_work(&self.all_sessions, &path) {
            if archive {
                self.set_session_archived(path, cx);
            } else {
                self.set_session_review(path, cx);
            }
            return;
        }

        self.cover_native_workspace_surface(cx);
        self.pending_archive = Some(PendingArchive {
            path,
            return_focus: window.focused(cx),
        });
        self.sheet_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn stop_and_archive_pending_session(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.close_archive_confirmation(window, cx) else {
            return;
        };
        self.send(RuntimeCommand::StopSessionFamily { path: path.clone() });
        self.set_session_archived(path, cx);
    }

    pub(super) fn close_archive_confirmation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PathBuf> {
        let pending = self.pending_archive.take()?;
        pending
            .return_focus
            .unwrap_or_else(|| self.composer_focus.clone())
            .focus(window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
        Some(pending.path)
    }
}

fn session_family_has_active_work(sessions: &[SessionSummary], path: &Path) -> bool {
    session_family_for_path(sessions, path)
        .is_some_and(|family| family.into_iter().any(|session| session.is_running))
}

pub(super) fn session_event_affects_archived_rail(
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
mod tests {
    use std::{path::PathBuf, time::SystemTime};

    use super::*;
    use crate::sessions::UsageSummary;

    fn session(id: &str, parent: Option<&str>, archived: bool, running: bool) -> SessionSummary {
        SessionSummary::from_cached(
            id.into(),
            PathBuf::from(format!("/sessions/{id}.jsonl")),
            PathBuf::from("/project"),
            id.into(),
            String::new(),
            String::new(),
            parent.map(str::to_owned),
            SystemTime::now(),
            0,
            UsageSummary::default(),
            archived,
            running,
            String::new(),
        )
    }

    #[test]
    fn active_work_includes_recursive_descendants() {
        let root = session("root", None, false, false);
        let child = session("child", Some("root"), false, false);
        let grandchild = session("grandchild", Some("child"), false, true);
        let sessions = [root.clone(), child, grandchild];

        assert!(session_family_has_active_work(&sessions, &root.path));
    }

    #[test]
    fn only_archived_family_events_invalidate_the_archived_rail() {
        let active = session("active", None, false, false);
        let archived = session("archived", None, true, false);
        let child = session("archived-child", Some("archived"), false, false);
        let sessions = [active.clone(), archived, child.clone()];

        assert!(!session_event_affects_archived_rail(
            &sessions,
            &session_target(&active.path),
            Some(&active.path),
        ));
        assert!(session_event_affects_archived_rail(
            &sessions,
            &session_target(&child.path),
            None,
        ));
    }
}
