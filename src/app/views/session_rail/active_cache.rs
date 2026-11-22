use std::{cell::RefCell, cmp::Reverse, collections::HashMap, rc::Rc};

use gpui::ListState;

use super::{
    ActiveSessionItem, FarcasterApp, RailSessionIdentity, SessionRailItem, SessionRailKind,
    active_item_identity, folders::FolderRow, reconcile_list_rows,
};
use crate::{
    app::infrastructure::performance::{OperationKind, OperationTiming},
    sessions::SessionKey,
    utility::persistent_vec::PersistentVec,
};

#[derive(Clone)]
pub(in crate::app::views) struct PreparedActiveRows {
    pub(super) rows: PersistentVec<Rc<FolderRow>>,
    pub(super) active_count: usize,
    pub(super) archived_count: usize,
    unfiled_count: usize,
    manual_order: Rc<HashMap<i64, usize>>,
    neighbors: Rc<RefCell<SelectableNeighbors>>,
}

impl FarcasterApp {
    pub(in crate::app::views) fn prepare_active_rows(
        &self,
        list: &ListState,
        identities: &RefCell<PersistentVec<String>>,
    ) -> PreparedActiveRows {
        let _timing = OperationTiming::new(
            OperationKind::SidebarSessionLists,
            self.sessions.visible.len(),
        );
        let lists = self.active_rail_lists(self.sessions.project_filter.as_deref());
        let active_count = lists.active.len();
        let mut rows = self.session_folder_rows(lists.active);
        if !self.settings.group_sessions_by_project {
            rows.push(FolderRow::New);
        }
        reconcile_list_rows(
            list,
            identities,
            rows.iter()
                .map(|row| match row {
                    FolderRow::Session(item) => active_item_identity(item),
                    FolderRow::Header(folder) => format!("folder:{}", folder.id),
                    FolderRow::Project { path, .. } => format!("project:{}", path.display()),
                    FolderRow::New => "new-folder".into(),
                })
                .collect(),
        );
        let rows: PersistentVec<_> = rows.into_iter().map(Rc::new).collect();
        let mut neighbors = SelectableNeighbors::default();
        for row in rows.iter() {
            neighbors.insert_before(row, None);
        }
        PreparedActiveRows {
            neighbors: Rc::new(RefCell::new(neighbors)),
            unfiled_count: rows
                .iter()
                .take_while(|row| matches!(row.as_ref(), FolderRow::Session(_)))
                .count(),
            rows,
            active_count,
            archived_count: lists.archived_count,
            manual_order: Rc::new(
                self.sessions
                    .order
                    .iter()
                    .enumerate()
                    .map(|(i, id)| (*id, i))
                    .collect(),
            ),
        }
    }
}

// Keep neighboring selectable rows attached by identity. Splicing the first row in a
// large group leaves the remaining links valid, including those in later groups.
#[derive(Default)]
struct SelectableNeighbors {
    rows: HashMap<RailSessionIdentity, Neighbor>,
    last: Option<RailSessionIdentity>,
}

struct Neighbor {
    row: Rc<FolderRow>,
    previous: Option<RailSessionIdentity>,
    next: Option<RailSessionIdentity>,
}

fn selectable_item(row: &FolderRow) -> Option<&ActiveSessionItem> {
    match row {
        FolderRow::Session(item) if !matches!(item.as_ref(), ActiveSessionItem::Draft(draft) if !draft.submitted) => {
            Some(item)
        }
        _ => None,
    }
}

impl SelectableNeighbors {
    fn insert_before(&mut self, row: &Rc<FolderRow>, next: Option<RailSessionIdentity>) {
        let Some(item) = selectable_item(row) else {
            return;
        };
        let id = RailSessionIdentity::for_item(item);
        let previous = if let Some(next_id) = next.as_ref() {
            self.rows
                .get_mut(next_id)
                .and_then(|n| n.previous.replace(id.clone()))
        } else {
            self.last.replace(id.clone())
        };
        if let Some(prev) = previous.as_ref().and_then(|prev| self.rows.get_mut(prev)) {
            prev.next = Some(id.clone());
        }
        self.rows.insert(
            id,
            Neighbor {
                row: row.clone(),
                previous,
                next,
            },
        );
    }

    fn remove(&mut self, item: &ActiveSessionItem) {
        let Some(Neighbor { previous, next, .. }) =
            self.rows.remove(&RailSessionIdentity::for_item(item))
        else {
            return;
        };
        if let Some(prev) = previous.as_ref().and_then(|prev| self.rows.get_mut(prev)) {
            prev.next = next.clone();
        }
        if let Some(next) = next.as_ref().and_then(|next| self.rows.get_mut(next)) {
            next.previous = previous;
        } else {
            self.last = previous;
        }
    }

    fn get(&self, id: &RailSessionIdentity) -> Option<&ActiveSessionItem> {
        selectable_item(&self.rows.get(id)?.row)
    }
}

impl PreparedActiveRows {
    #[cfg(test)]
    pub(super) fn visible_items(&self) -> impl Iterator<Item = &ActiveSessionItem> {
        self.rows.iter().filter_map(|row| match &**row {
            FolderRow::Session(item) => Some(item.as_ref()),
            _ => None,
        })
    }

    pub(super) fn replacement(&self, selected: &RailSessionIdentity) -> Option<ActiveSessionItem> {
        let neighbors = self.neighbors.borrow();
        let current = neighbors.rows.get(selected)?;
        neighbors
            .get(current.next.as_ref().or(current.previous.as_ref())?)
            .cloned()
    }

    pub(super) fn selectable(&self, id: &RailSessionIdentity) -> Option<ActiveSessionItem> {
        self.neighbors.borrow().get(id).cloned()
    }

    pub(super) fn last_drop_target(&self, app: &FarcasterApp) -> Option<(usize, i64)> {
        if app.settings.group_sessions_by_project || !app.sessions.folders.folders.is_empty() {
            return None;
        }
        let index = self.active_count.checked_sub(1)?;
        let FolderRow::Session(item) = &**self.rows.get(index)? else {
            return None;
        };
        Some((index, item.app_session_id()))
    }

    fn project_headers(&self) -> impl Iterator<Item = (usize, &std::path::Path, &str)> {
        let mut index = 0;
        std::iter::from_fn(move || {
            let start = index;
            let FolderRow::Project {
                path,
                label,
                count,
                collapsed,
            } = &**self.rows.get(index)?
            else {
                return None;
            };
            index += 1 + if *collapsed { 0 } else { *count };
            Some((start, path.as_path(), label.as_str()))
        })
    }

    fn relabel_projects(&mut self, list: &ListState) {
        let paths = self
            .project_headers()
            .map(|(_, path, _)| path.to_owned())
            .collect();
        let updates = self
            .project_headers()
            .zip(super::folders::project_labels(paths))
            .filter_map(|((index, _, old), new)| (new != old).then_some((index, new)))
            .collect::<Vec<_>>();
        for (index, new) in updates {
            let mut header = (*self.rows[index]).clone();
            if let FolderRow::Project { label, .. } = &mut header {
                *label = new;
            }
            self.rows.splice(index..index + 1, [Rc::new(header)]);
            list.remeasure_items(index..index + 1);
        }
    }

    // Equal nonpositive IDs use catalog insertion order, which cannot be recovered from the ID.
    // Draft promotion can change two identities at once. Reprepare those transitions.
    pub(in crate::app::views) fn patch_archive(
        &mut self,
        app: &FarcasterApp,
        key: Option<&SessionKey>,
        draft_id: Option<&str>,
        list: &ListState,
        identities: &RefCell<PersistentVec<String>>,
    ) -> bool {
        if key.is_some() && draft_id.is_some() {
            return false;
        }
        let (item, archived) = if let Some(draft_id) = draft_id {
            let Some(draft) = app
                .sessions
                .drafts
                .iter()
                .find(|draft| draft.id == draft_id)
            else {
                return false;
            };
            if draft.app_session_id <= 0
                || app
                    .sessions
                    .visible
                    .by_app_id(draft.app_session_id)
                    .is_some()
            {
                return false;
            }
            let mut presented = draft.clone();
            app.present_rail_draft(&mut presented);
            (ActiveSessionItem::Draft(presented), draft.archived)
        } else if let Some(key) = key {
            let Some(row) = app.sessions.visible.by_key(key) else {
                return false;
            };
            if !app.sessions.visible.is_root_row(row) {
                return true;
            }
            if row.app_session_id <= 0
                || app
                    .sessions
                    .drafts
                    .iter()
                    .any(|draft| draft.app_session_id == row.app_session_id)
            {
                return false;
            }
            (
                ActiveSessionItem::Session(SessionRailItem {
                    session: row.clone(),
                    kind: SessionRailKind::Project,
                }),
                row.archived,
            )
        } else {
            return true;
        };
        if app
            .sessions
            .project_filter
            .as_deref()
            .is_some_and(|filter| filter != item.project())
        {
            return true;
        }

        let grouped = app.settings.group_sessions_by_project;
        let folder = match &item {
            ActiveSessionItem::Draft(draft) => crate::app::session_folders::draft_folder(
                draft,
                &app.sessions.folders,
                &app.sessions.pending_folder_drafts,
            ),
            _ => app.sessions.folders.folder_for(item.app_session_id()),
        };
        let mut start = 0;
        let end;
        let mut header = None;
        if grouped || folder.is_some() {
            let mut position = self.unfiled_count;
            let found = loop {
                let Some(row) = self.rows.get(position) else {
                    break None;
                };
                let (target, count, collapsed) = match &**row {
                    FolderRow::Project {
                        path,
                        count,
                        collapsed,
                        ..
                    } => (grouped && path == item.project(), *count, *collapsed),
                    FolderRow::Header(h) => {
                        (!grouped && Some(h.id) == folder, h.count, h.collapsed)
                    }
                    _ => break None,
                };
                if target {
                    break Some((position, count, collapsed));
                }
                position += 1 + if collapsed { 0 } else { count };
            };
            let (index, count, collapsed) = match found {
                Some(found) => found,
                None if grouped && !archived => {
                    let path = item.project();
                    let rank = |path: &std::path::Path| {
                        app.sessions
                            .folders
                            .project_order
                            .iter()
                            .position(|p| p == path)
                            .unwrap_or(usize::MAX)
                    };
                    let position = self
                        .project_headers()
                        .find(|(_, project, _)| (rank(project), *project) > (rank(path), path))
                        .map_or(self.rows.len(), |(i, _, _)| i);
                    let collapsed = app.sessions.collapsed_projects.contains(path);
                    identities
                        .borrow_mut()
                        .insert(position, format!("project:{}", path.display()));
                    self.rows.insert(
                        position,
                        Rc::new(FolderRow::Project {
                            path: path.to_owned(),
                            label: String::new(),
                            count: 0,
                            collapsed,
                        }),
                    );
                    list.splice(position..position, 1);
                    (position, 0, collapsed)
                }
                None => return false,
            };
            start = index + 1;
            end = start + if collapsed { 0 } else { count };
            header = Some((index, collapsed));
        } else {
            end = self.unfiled_count;
        }

        let key_for = |item: &ActiveSessionItem| {
            (
                !matches!(item, ActiveSessionItem::Draft(draft) if !draft.submitted),
                self.manual_order.get(&item.app_session_id()).copied(),
                Reverse(item.app_session_id()),
            )
        };
        if !header.is_some_and(|(_, collapsed)| collapsed) {
            let key = key_for(&item);
            let mut low = start;
            let mut high = end;
            while low < high {
                let mid = low + (high - low) / 2;
                let Some(FolderRow::Session(other)) = self.rows.get(mid).map(Rc::as_ref) else {
                    return false;
                };
                if key_for(other) < key {
                    low = mid + 1;
                } else {
                    high = mid;
                }
            }
            if archived {
                if low == end
                    || !matches!(&*self.rows[low], FolderRow::Session(other) if other.app_session_id() == item.app_session_id())
                {
                    return false;
                }
                self.neighbors.borrow_mut().remove(&item);
                self.rows.splice(low..low + 1, []);
                identities.borrow_mut().remove(low);
                list.splice(low..low + 1, 0);
            } else {
                identities
                    .borrow_mut()
                    .insert(low, active_item_identity(&item));
                let restored = Rc::new(FolderRow::Session(Box::new(item)));
                if selectable_item(&restored).is_some() {
                    let next = self
                        .rows
                        .iter_range(low..self.rows.len())
                        .find_map(|row| selectable_item(row).map(RailSessionIdentity::for_item));
                    self.neighbors.borrow_mut().insert_before(&restored, next);
                }
                self.rows.insert(low, restored);
                list.splice(low..low, 1);
            }
        }
        if let Some((index, _)) = header {
            let mut row = (*self.rows[index]).clone();
            let count = match &mut row {
                FolderRow::Project { count, .. } => count,
                FolderRow::Header(h) => &mut h.count,
                _ => unreachable!(),
            };
            if archived {
                *count -= 1;
            } else {
                *count += 1;
            }
            let relabel = grouped && ((*count == 1 && !archived) || *count == 0);
            if grouped && *count == 0 {
                self.rows.remove(index);
                identities.borrow_mut().remove(index);
                list.splice(index..index + 1, 0);
            } else {
                self.rows.splice(index..index + 1, [Rc::new(row)]);
                list.remeasure_items(index..index + 1);
            }
            if relabel {
                self.relabel_projects(list);
            }
        }
        if header.is_none() {
            if archived {
                self.unfiled_count -= 1;
            } else {
                self.unfiled_count += 1;
            }
        }
        if archived {
            self.active_count -= 1;
            self.archived_count += 1;
        } else {
            self.archived_count -= 1;
            self.active_count += 1;
        }
        true
    }
}
