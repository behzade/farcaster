use gpui::{Context, Window};

use super::FarcasterApp;
use crate::sessions::DraftSession;
#[cfg(test)]
pub(crate) use crate::sessions::SessionFolder;
pub(crate) use crate::sessions::{FolderDestination, SessionFolders};

#[derive(Clone, Copy)]
pub(in crate::app) struct FolderEdit {
    pub(in crate::app) id: Option<u64>,
    pub(in crate::app) session: Option<i64>,
}

impl FarcasterApp {
    pub(in crate::app) fn move_session_to_folder(
        &mut self,
        session: i64,
        path: std::path::PathBuf,
        destination: FolderDestination,
        archived: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if destination == FolderDestination::Archived {
            self.request_session_archive(path, true, window, cx);
            return;
        }
        let folder = match destination {
            FolderDestination::Folder(id) => Some(id),
            _ => None,
        };
        if self.assign_session_folder(session, folder, cx) && archived {
            self.request_session_archive(path, false, window, cx);
        }
    }

    pub(in crate::app) fn assign_session_folder(
        &mut self,
        session: i64,
        folder: Option<u64>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.sessions.folders.folder_for(session) == folder {
            return true;
        }
        let mut next = self.sessions.folders.clone();
        next.assign(session, folder);
        self.save_session_folders(next, cx)
    }

    pub(in crate::app) fn save_session_folders(
        &mut self,
        next: SessionFolders,
        cx: &mut Context<Self>,
    ) -> bool {
        self.sessions.folders = next;
        self.save_session_state(cx);
        true
    }

    pub(in crate::app) fn begin_folder_edit(
        &mut self,
        id: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = id
            .and_then(|id| self.sessions.folders.folders.iter().find(|f| f.id == id))
            .map(|f| f.name.clone())
            .unwrap_or_default();
        self.sessions.editing_title = None;
        self.sessions.editing_folder = Some(FolderEdit { id, session: None });
        self.sessions.title_input.update(cx, |input, cx| {
            input.set_placeholder("Folder name", window, cx);
            input.set_value(name.clone(), window, cx);
            input.set_selected_range(0..name.len(), cx);
        });
        self.sessions.pending_title_focus = true;
        self.notify_session_rail(cx);
        cx.notify();
    }

    pub(in crate::app) fn commit_folder_edit(&mut self, cx: &mut Context<Self>) {
        let Some(FolderEdit { id, session }) = self.sessions.editing_folder.take() else {
            return;
        };
        let name = self.sessions.title_input.read(cx).value().trim().to_owned();
        if !name.is_empty() {
            let mut next = self.sessions.folders.clone();
            if let Some(id) = id {
                if let Some(folder) = next.folders.iter_mut().find(|f| f.id == id) {
                    folder.name = name;
                }
            } else {
                next.create(name, session);
            }
            self.save_session_folders(next, cx);
        }
        self.notify_session_rail(cx);
        cx.notify();
    }
}

#[cfg(test)]
#[path = "session_folders_tests.rs"]
mod tests;
