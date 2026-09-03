use std::path::PathBuf;

use gpui::{Context, Window};

use super::{FarcasterApp, SessionTitleEdit};
use crate::{
    runtime::RuntimeCommand,
    sessions::{SessionSummary, normalize_session_path},
};

impl FarcasterApp {
    pub(in crate::app) fn begin_session_title_edit(
        &mut self,
        path: PathBuf,
        project: PathBuf,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing_session_title = Some(SessionTitleEdit {
            path,
            project,
            original: title.clone(),
        });
        self.session_title_input.update(cx, |input, cx| {
            input.set_value(title.clone(), window, cx);
            input.set_selected_range(0..title.len(), cx);
        });
        self.pending_session_title_focus = true;
        self.notify_session_rail(cx);
        cx.notify();
    }

    pub(in crate::app) fn commit_session_title_edit(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.editing_session_title.take() else {
            return;
        };
        let title = self.session_title_input.read(cx).value().trim().to_owned();
        if title.is_empty() || title == edit.original {
            self.notify_session_rail(cx);
            cx.notify();
            return;
        }
        let active_path = self
            .snapshot
            .session
            .as_ref()
            .and_then(|session| session.session_file.as_deref())
            .map(PathBuf::from)
            .map(|path| normalize_session_path(&path));
        let edited_path = normalize_session_path(&edit.path);
        self.pending_session_titles
            .insert(edited_path.clone(), title.clone());
        set_session_title(
            &mut self.sessions,
            &mut self.all_sessions,
            &edited_path,
            &title,
        );

        if active_path.as_deref() == Some(edited_path.as_path()) && !self.snapshot.history_preview {
            self.send(RuntimeCommand::SetSessionName(title));
        } else {
            let target = self.backend_target_for_path(&edit.path);
            self.send(RuntimeCommand::RenameSession {
                path: edit.path,
                harness: target.harness,
                session_id: target.id,
                project: edit.project,
                name: title,
            });
        }
        self.notify_session_rail(cx);
        cx.notify();
    }

    pub(in crate::app) fn cancel_session_title_edit(&mut self, cx: &mut Context<Self>) {
        if self.editing_session_title.take().is_some() {
            self.notify_session_rail(cx);
            cx.notify();
        }
    }

    pub(in crate::app) fn reconcile_pending_session_titles(
        &mut self,
        sessions: &mut [SessionSummary],
        all_sessions: &mut [SessionSummary],
    ) {
        self.pending_session_titles.retain(|path, pending_title| {
            !all_sessions.iter().chain(sessions.iter()).any(|session| {
                normalize_session_path(&session.path) == *path && session.title == *pending_title
            })
        });

        for (path, title) in &self.pending_session_titles {
            set_session_title(sessions, all_sessions, path, title);
        }
    }
}

fn set_session_title(
    sessions: &mut [SessionSummary],
    all_sessions: &mut [SessionSummary],
    path: &std::path::Path,
    title: &str,
) {
    for session in sessions.iter_mut().chain(all_sessions) {
        if normalize_session_path(&session.path) == path {
            session.title = title.to_owned();
        }
    }
}
