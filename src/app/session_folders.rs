use std::collections::BTreeMap;

use gpui::{Context, Window};
use serde::{Deserialize, Serialize};

use super::{FarcasterApp, persistence::StateStore};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct SessionFolders {
    pub(crate) folders: Vec<SessionFolder>,
    pub(crate) membership: BTreeMap<i64, u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct SessionFolder {
    pub(crate) id: u64,
    pub(crate) name: String,
}

#[derive(Clone, Copy)]
pub(in crate::app) struct FolderEdit {
    pub(in crate::app) id: Option<u64>,
    pub(in crate::app) session: Option<i64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FolderDestination {
    Active,
    Folder(u64),
    Archived,
}

impl SessionFolders {
    pub(crate) fn destination(&self, session: i64, archived: bool) -> FolderDestination {
        if archived {
            FolderDestination::Archived
        } else {
            self.folder_for(session)
                .map(FolderDestination::Folder)
                .unwrap_or(FolderDestination::Active)
        }
    }

    pub(crate) fn destinations(&self) -> Vec<(FolderDestination, String)> {
        std::iter::once((FolderDestination::Active, "Active".to_owned()))
            .chain(
                self.folders
                    .iter()
                    .map(|folder| (FolderDestination::Folder(folder.id), folder.name.clone())),
            )
            .chain(std::iter::once((
                FolderDestination::Archived,
                "Archived".to_owned(),
            )))
            .collect()
    }

    pub(crate) fn create(&mut self, name: String, session: Option<i64>) {
        let id = self.folders.iter().map(|f| f.id).max().unwrap_or(0) + 1;
        self.folders.push(SessionFolder { id, name });
        if let Some(session) = session {
            self.assign(session, Some(id));
        }
    }

    pub(crate) fn folder_for(&self, session: i64) -> Option<u64> {
        self.membership
            .get(&session)
            .copied()
            .filter(|id| self.folders.iter().any(|f| f.id == *id))
    }

    pub(crate) fn assign(&mut self, session: i64, folder: Option<u64>) {
        if session <= 0 {
            return;
        }
        match folder {
            Some(id) if self.folders.iter().any(|f| f.id == id) => {
                self.membership.insert(session, id);
            }
            None => {
                self.membership.remove(&session);
            }
            _ => {}
        }
    }

    pub(crate) fn remove(&mut self, id: u64) {
        self.folders.retain(|f| f.id != id);
        self.membership.retain(|_, folder| *folder != id);
    }
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
        if self.session_folders.folder_for(session) == folder {
            return true;
        }
        let mut next = self.session_folders.clone();
        next.assign(session, folder);
        self.save_session_folders(next, cx)
    }

    pub(in crate::app) fn save_session_folders(
        &mut self,
        next: SessionFolders,
        cx: &mut Context<Self>,
    ) -> bool {
        match StateStore::open().and_then(|store| store.save_session_folders(&next)) {
            Ok(()) => {
                self.session_folders = next;
                self.notify_session_rail(cx);
                true
            }
            Err(error) => {
                self.sessions_error = Some(error);
                self.notify_session_rail(cx);
                false
            }
        }
    }

    pub(in crate::app) fn begin_folder_edit(
        &mut self,
        id: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = id
            .and_then(|id| self.session_folders.folders.iter().find(|f| f.id == id))
            .map(|f| f.name.clone())
            .unwrap_or_default();
        self.editing_session_title = None;
        self.editing_folder = Some(FolderEdit { id, session: None });
        self.session_title_input.update(cx, |input, cx| {
            input.set_placeholder("Folder name", window, cx);
            input.set_value(name.clone(), window, cx);
            input.set_selected_range(0..name.len(), cx);
        });
        self.pending_session_title_focus = true;
        self.notify_session_rail(cx);
        cx.notify();
    }

    pub(in crate::app) fn commit_folder_edit(&mut self, cx: &mut Context<Self>) {
        let Some(FolderEdit { id, session }) = self.editing_folder.take() else {
            return;
        };
        let name = self.session_title_input.read(cx).value().trim().to_owned();
        if !name.is_empty() {
            let mut next = self.session_folders.clone();
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
