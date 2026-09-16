use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
        let id = self
            .folders
            .iter()
            .map(|folder| folder.id)
            .max()
            .unwrap_or(0)
            + 1;
        self.folders.push(SessionFolder { id, name });
        if let Some(session) = session {
            self.assign(session, Some(id));
        }
    }

    pub(crate) fn folder_for(&self, session: i64) -> Option<u64> {
        self.membership
            .get(&session)
            .copied()
            .filter(|id| self.folders.iter().any(|folder| folder.id == *id))
    }

    pub(crate) fn assign(&mut self, session: i64, folder: Option<u64>) {
        if session <= 0 {
            return;
        }
        match folder {
            Some(id) if self.folders.iter().any(|folder| folder.id == id) => {
                self.membership.insert(session, id);
            }
            None => {
                self.membership.remove(&session);
            }
            _ => {}
        }
    }

    pub(crate) fn remove(&mut self, id: u64) {
        self.folders.retain(|folder| folder.id != id);
        self.membership.retain(|_, folder| *folder != id);
    }
}
