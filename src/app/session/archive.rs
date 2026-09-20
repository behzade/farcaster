use std::path::{Path, PathBuf};

use gpui::{Context, FocusHandle, Window};

use super::FarcasterApp;
use crate::{
    app::composer::sessions::session_target,
    projects::DraftSession,
    runtime::RuntimeCommand,
    sessions::{SessionSummary, root_session_for_path},
};

/// Where a chat's archived state lives. A chat that has never been written to
/// has no session file yet, so it keeps that state on its registry record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::app) enum ChatArchiveTarget {
    Draft(String),
    Session(PathBuf),
}

pub(in crate::app) fn chat_archive_target(
    drafts: &[DraftSession],
    sessions: &[SessionSummary],
    app_session_id: i64,
) -> Option<ChatArchiveTarget> {
    if let Some(draft) = drafts
        .iter()
        .find(|draft| draft.app_session_id == app_session_id)
    {
        return Some(ChatArchiveTarget::Draft(draft.id.clone()));
    }
    sessions
        .iter()
        .find(|session| session.app_session_id == app_session_id)
        .map(|session| ChatArchiveTarget::Session(session.path.clone()))
}

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
        let active = self.session_family_has_active_work(&path);
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
        self.sessions.pending_archive = Some(pending);
        cx.notify();
    }

    /// File a chat away, or bring it back, by the identity the rail shows.
    /// Every chat resolves: one that was written to a session takes the session
    /// with it, and one that was not keeps its state on its own record.
    pub(in crate::app) fn request_chat_archive(
        &mut self,
        app_session_id: i64,
        archive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match chat_archive_target(
            &self.sessions.drafts,
            &self.sessions.visible,
            app_session_id,
        ) {
            Some(ChatArchiveTarget::Draft(id)) => {
                self.request_draft_archive(id, archive, window, cx);
            }
            Some(ChatArchiveTarget::Session(path)) => {
                self.request_session_archive(path, archive, window, cx);
            }
            None => {}
        }
    }

    pub(in crate::app) fn request_session_archive_and_advance(
        &mut self,
        path: PathBuf,
        next_app_session_id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_session_archive(path, true, window, cx);
        if let Some(pending) = self.sessions.pending_archive.as_mut() {
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
        self.send(stop_and_archive_command(path), cx);
        if let Some(id) = next_app_session_id {
            self.select_visible_app_session(id, window, cx);
        }
    }

    pub(in crate::app) fn close_archive_confirmation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(PathBuf, Option<i64>)> {
        let pending = self.sessions.pending_archive.take()?;
        self.restore_overlay_focus(pending.return_focus.clone(), &pending.focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
        Some((pending.path, pending.next_app_session_id))
    }
}

fn stop_and_archive_command(path: PathBuf) -> RuntimeCommand {
    RuntimeCommand::StopSessionFamily { path }
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
