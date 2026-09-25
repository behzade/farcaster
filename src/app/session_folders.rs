use std::{collections::HashSet, path::PathBuf};

use gpui::{Context, Window};

use super::FarcasterApp;
use crate::sessions::DraftSession;
#[cfg(test)]
pub(crate) use crate::sessions::SessionFolder;
pub(crate) use crate::sessions::{FolderDestination, SessionFolders};
use crate::sessions::{SessionSummary, session_family_for_path};

#[derive(Clone, Copy)]
pub(in crate::app) struct FolderEdit {
    pub(in crate::app) id: Option<u64>,
    pub(in crate::app) session: Option<i64>,
}

/// The chats a folder holds, and every path that has to leave the rail with
/// them. Root paths are deduplicated so a family is deleted once even though
/// each of its subagents resolves to it.
#[derive(Debug, Default, Eq, PartialEq)]
pub(in crate::app) struct FolderDeletion {
    pub(in crate::app) roots: Vec<PathBuf>,
    pub(in crate::app) family_paths: HashSet<PathBuf>,
    pub(in crate::app) drafts: Vec<String>,
}

pub(in crate::app) fn folder_deletion(
    sessions: &[SessionSummary],
    drafts: &[DraftSession],
    folders: &SessionFolders,
    folder: u64,
) -> FolderDeletion {
    let mut deletion = FolderDeletion::default();
    for session in sessions.iter().filter(|session| !session.archived) {
        if folders.folder_for_session(session.app_session_id, &session.project) != Some(folder) {
            continue;
        }
        let Some(family) = session_family_for_path(sessions, &session.path) else {
            continue;
        };
        if let Some(root) = family.first()
            && !deletion.roots.contains(&root.path)
        {
            deletion.roots.push(root.path.clone());
        }
        deletion
            .family_paths
            .extend(family.into_iter().map(|session| session.path.clone()));
    }
    deletion.drafts = drafts
        .iter()
        .filter(|draft| {
            folders.folder_for_session(draft.app_session_id, &draft.project) == Some(folder)
        })
        .map(|draft| draft.id.clone())
        .collect();
    deletion
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

    pub(in crate::app) fn sync_project_folders(&mut self, cx: &mut Context<Self>) {
        let mut projects = Vec::new();
        let mut live_projects = Vec::new();
        for session in &self.sessions.all {
            live_projects.push(session.project.clone());
            if !session.archived {
                projects.push(session.project.clone());
            }
        }
        for draft in &self.sessions.drafts {
            projects.push(draft.project.clone());
            live_projects.push(draft.project.clone());
        }
        projects.sort();
        projects.dedup();
        live_projects.sort();
        live_projects.dedup();
        let chats = self
            .sessions
            .all
            .iter()
            .map(|session| session.app_session_id)
            .chain(
                self.sessions
                    .drafts
                    .iter()
                    .map(|draft| draft.app_session_id),
            )
            .collect::<Vec<_>>();
        let mut next = self.sessions.folders.clone();
        let mut changed = false;
        for project in &projects {
            changed |= next.ensure_project_folder(project, false);
        }
        changed |= next.prune_project_folders(&chats, &live_projects);
        if changed {
            self.save_session_folders(next, cx);
        }
    }

    pub(in crate::app) fn ensure_open_project_folder(
        &mut self,
        project: &std::path::Path,
        cx: &mut Context<Self>,
    ) {
        let mut next = self.sessions.folders.clone();
        if next.ensure_project_folder(project, true) {
            self.save_session_folders(next, cx);
        }
    }

    pub(in crate::app) fn set_folder_collapsed(
        &mut self,
        id: u64,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut next = self.sessions.folders.clone();
        if !next.set_collapsed(id, collapsed) {
            return false;
        }
        self.save_session_folders(next, cx)
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
