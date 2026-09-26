use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

pub const FOLDER_COLOR_COUNT: usize = 8;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SessionFolders {
    pub folders: Vec<SessionFolder>,
    pub membership: BTreeMap<i64, u64>,
    #[serde(default)]
    pub session_colors: BTreeMap<i64, u8>,
    #[serde(default)]
    pub project_colors: BTreeMap<PathBuf, u8>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SessionFolder {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub color: Option<u8>,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub project: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FolderDestination {
    Active,
    Folder(u64),
    Archived,
}

impl SessionFolders {
    pub fn destination(&self, session: i64, archived: bool) -> FolderDestination {
        if archived {
            FolderDestination::Archived
        } else {
            self.folder_for(session)
                .map(FolderDestination::Folder)
                .unwrap_or(FolderDestination::Active)
        }
    }

    pub fn destinations(&self) -> Vec<(FolderDestination, String)> {
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

    pub fn create(&mut self, name: String, session: Option<i64>) {
        let id = self
            .folders
            .iter()
            .map(|folder| folder.id)
            .max()
            .unwrap_or(0)
            + 1;
        self.folders.push(SessionFolder {
            id,
            name,
            color: None,
            collapsed: false,
            pinned: false,
            project: None,
        });
        if let Some(session) = session {
            self.assign(session, Some(id));
        }
    }

    pub fn set_color(&mut self, id: u64, color: Option<u8>) -> bool {
        let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == id) else {
            return false;
        };
        let color = color.map(|color| color % u8::try_from(FOLDER_COLOR_COUNT).unwrap_or(1));
        if folder.color == color {
            return false;
        }
        folder.color = color;
        true
    }

    pub fn set_collapsed(&mut self, id: u64, collapsed: bool) -> bool {
        let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == id) else {
            return false;
        };
        if folder.collapsed == collapsed {
            return false;
        }
        folder.collapsed = collapsed;
        true
    }

    pub fn session_color(&self, session: i64) -> Option<u8> {
        self.session_colors.get(&session).copied()
    }

    pub fn set_session_color(&mut self, session: i64, color: Option<u8>) -> bool {
        if session <= 0 {
            return false;
        }
        match color {
            Some(color) => {
                let color = color % u8::try_from(FOLDER_COLOR_COUNT).unwrap_or(1);
                self.session_colors.insert(session, color) != Some(color)
            }
            None => self.session_colors.remove(&session).is_some(),
        }
    }

    pub fn folder_for(&self, session: i64) -> Option<u64> {
        self.membership
            .get(&session)
            .copied()
            .filter(|id| self.folders.iter().any(|folder| folder.id == *id))
    }

    /// Old rail builds stored automatic project groups as custom folders.
    /// Keep manually assigned groups as folders, preserving their IDs and members.
    pub fn separate_project_groups(&mut self) {
        self.folders.retain_mut(|folder| {
            if folder.project.take().is_none() {
                return true;
            }
            self.membership.values().any(|id| *id == folder.id)
        });
    }

    pub fn set_project_color(&mut self, project: PathBuf, color: Option<u8>) -> bool {
        match color {
            Some(color) => {
                let color = color % u8::try_from(FOLDER_COLOR_COUNT).unwrap_or(1);
                self.project_colors.insert(project, color) != Some(color)
            }
            None => self.project_colors.remove(&project).is_some(),
        }
    }

    pub fn assign(&mut self, session: i64, folder: Option<u64>) {
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

    pub fn remove(&mut self, id: u64) {
        self.folders.retain(|folder| folder.id != id);
        self.membership.retain(|_, folder| *folder != id);
    }
}

#[cfg(test)]
#[path = "folders_tests.rs"]
mod tests;
