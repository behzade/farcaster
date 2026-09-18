use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

pub const FOLDER_COLOR_COUNT: usize = 8;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SessionFolders {
    pub folders: Vec<SessionFolder>,
    pub membership: BTreeMap<i64, u64>,
    #[serde(default)]
    pub session_colors: BTreeMap<i64, u8>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SessionFolder {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub color: u8,
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
        let color = self.next_color();
        self.folders.push(SessionFolder {
            id,
            name,
            color,
            collapsed: false,
            pinned: false,
            project: None,
        });
        if let Some(session) = session {
            self.assign(session, Some(id));
        }
    }

    pub fn set_color(&mut self, id: u64, color: u8) -> bool {
        let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == id) else {
            return false;
        };
        let color = color % u8::try_from(FOLDER_COLOR_COUNT).unwrap_or(1);
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

    pub fn folder_for_project(&self, project: &Path) -> Option<u64> {
        self.folders
            .iter()
            .find(|folder| folder.project.as_deref() == Some(project))
            .map(|folder| folder.id)
    }

    pub fn folder_for_session(&self, session: i64, project: &Path) -> Option<u64> {
        self.folder_for(session)
            .or_else(|| self.folder_for_project(project))
    }

    pub fn ensure_project_folder(&mut self, project: &Path, pinned: bool) -> bool {
        if let Some(folder) = self
            .folders
            .iter_mut()
            .find(|folder| folder.project.as_deref() == Some(project))
        {
            if pinned && !folder.pinned {
                folder.pinned = true;
                return true;
            }
            return false;
        }
        let name = project
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .map_or_else(|| project.display().to_string(), str::to_owned);
        let id = self
            .folders
            .iter()
            .map(|folder| folder.id)
            .max()
            .unwrap_or(0)
            + 1;
        let color = self.next_color();
        self.folders.push(SessionFolder {
            id,
            name,
            color,
            collapsed: false,
            pinned,
            project: Some(project.to_path_buf()),
        });
        true
    }

    fn next_color(&self) -> u8 {
        (0..FOLDER_COLOR_COUNT)
            .find(|index| {
                !self
                    .folders
                    .iter()
                    .any(|folder| usize::from(folder.color) == *index)
            })
            .map_or_else(
                || u8::try_from(self.folders.len() % FOLDER_COLOR_COUNT).unwrap_or(0),
                |index| u8::try_from(index).unwrap_or(0),
            )
    }

    pub fn prune_project_folders(
        &mut self,
        chats: &[i64],
        live_projects: &[PathBuf],
    ) -> bool {
        let held = chats
            .iter()
            .filter_map(|chat| self.membership.get(chat).copied())
            .collect::<std::collections::HashSet<_>>();
        let before = self.folders.len();
        self.folders.retain(|folder| {
            let Some(project) = folder.project.as_deref() else {
                return true;
            };
            folder.pinned
                || held.contains(&folder.id)
                || live_projects.iter().any(|live| live == project)
        });
        if self.folders.len() == before {
            return false;
        }
        let live = self
            .folders
            .iter()
            .map(|folder| folder.id)
            .collect::<std::collections::HashSet<_>>();
        self.membership.retain(|_, folder| live.contains(folder));
        true
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
