use std::{collections::HashSet, path::PathBuf};

use gpui::{Context, FocusHandle, Window};

use super::FarcasterApp;
use crate::{
    app::session_folders::folder_deletion, runtime::RuntimeCommand,
    sessions::session_family_for_path,
};

const SESSION_MESSAGE: &str =
    "This permanently deletes the session and all of its subagent sessions. This cannot be undone.";

pub(in crate::app) struct PendingDelete {
    pub(in crate::app) focus: FocusHandle,
    roots: Vec<PathBuf>,
    family_paths: HashSet<PathBuf>,
    drafts: Vec<String>,
    folder: Option<u64>,
    message: &'static str,
    return_focus: Option<FocusHandle>,
}

impl PendingDelete {
    pub(in crate::app) fn title(&self) -> &'static str {
        if self.folder.is_some() {
            "Delete folder and its chats?"
        } else {
            "Delete session permanently?"
        }
    }

    pub(in crate::app) fn message(&self) -> &'static str {
        self.message
    }
}

impl FarcasterApp {
    pub(in crate::app) fn request_session_delete(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(family) = session_family_for_path(&self.sessions.all, &path) else {
            self.sessions.error = Some("The session is no longer available to delete".to_owned());
            self.notify_session_rail(cx);
            return;
        };
        let family_paths = family
            .into_iter()
            .map(|session| session.path.clone())
            .collect();
        self.open_delete_confirmation(
            vec![path.clone()],
            family_paths,
            Vec::new(),
            None,
            SESSION_MESSAGE,
            window,
            cx,
        );
    }

    pub(in crate::app) fn request_folder_delete(
        &mut self,
        folder: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sessions.deleting_folders.contains_key(&folder) {
            return;
        }
        let deletion = folder_deletion(
            &self.sessions.all,
            &self.sessions.drafts,
            &self.sessions.folders,
            folder,
        );
        if deletion.drafts.iter().any(|id| {
            self.sessions
                .drafts
                .iter()
                .any(|draft| draft.id == *id && draft.submitted)
                || crate::app::composer::submissions::has_pending_submission(
                    &self.composer.pending_submissions,
                    &crate::app::composer::sessions::draft_target(id),
                )
        }) {
            self.sessions.error =
                Some("Wait for pending chat submissions before deleting this folder".into());
            self.notify_session_rail(cx);
            return;
        }
        self.open_delete_confirmation(
            deletion.roots,
            deletion.family_paths,
            deletion.drafts,
            Some(folder),
            "This permanently deletes every chat in the folder and all of their subagent sessions. This cannot be undone.",
            window,
            cx,
        );
    }

    fn open_delete_confirmation(
        &mut self,
        roots: Vec<PathBuf>,
        family_paths: HashSet<PathBuf>,
        drafts: Vec<String>,
        folder: Option<u64>,
        message: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cover_native_workspace_surface(cx);
        let pending = PendingDelete {
            focus: cx.focus_handle(),
            roots,
            family_paths,
            drafts,
            folder,
            message,
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
        for draft in &pending.drafts {
            self.discard_draft(draft, window, cx);
        }
        if let Some(folder) = pending.folder {
            if pending.roots.is_empty() {
                let mut next = self.sessions.folders.clone();
                next.remove(folder);
                self.save_session_folders(next, cx);
            } else {
                self.sessions
                    .deleting_folders
                    .insert(folder, pending.family_paths);
            }
        }
        self.notify_session_rail(cx);
        for root in pending.roots {
            self.send(RuntimeCommand::DeleteSessionFamily { path: root }, cx);
        }
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
