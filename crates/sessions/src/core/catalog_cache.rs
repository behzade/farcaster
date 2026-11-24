use std::{
    cell::OnceCell,
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

use super::catalog::SessionRootIndex;
use crate::{NativeSessionIdentity, SessionSummary};

#[derive(Default)]
pub struct SessionCatalog {
    sessions: Vec<SessionSummary>,
    relationships: OnceCell<Relationships>,
    requested_archive_paths: HashSet<PathBuf>,
    #[cfg(any(test, feature = "test-support"))]
    relationship_builds: std::cell::Cell<usize>,
}

struct Relationships {
    slots: Vec<usize>,
    positions: Vec<usize>,
    by_path: HashMap<PathBuf, usize>,
    shared_paths: HashMap<PathBuf, Vec<usize>>,
    by_app_id: HashMap<i64, usize>,
    duplicate_app_ids: HashSet<i64>,
    unbound: Vec<UnboundRow>,
    running: Vec<usize>,
    parents: Vec<Option<usize>>,
    archived_roots: usize,
    children: Vec<Vec<usize>>,
    native_worker_parents: HashMap<NativeSessionIdentity, usize>,
}

struct UnboundRow {
    slot: usize,
    canonical_path: OnceCell<PathBuf>,
    canonical_project: OnceCell<PathBuf>,
}

fn legacy_identity_matches(legacy: &SessionSummary, stored: &SessionSummary) -> bool {
    legacy.harness == stored.harness
        && legacy.profile_id == stored.profile_id
        && (legacy.id == stored.id
            || legacy.id == legacy.path.to_string_lossy()
            || stored.id == stored.path.to_string_lossy())
}

fn same_stored_project(legacy: &SessionSummary, stored: &SessionSummary) -> bool {
    legacy.project == stored.project
        || crate::normalize_session_path(&legacy.project) == stored.project
}

fn only_match<T>(mut matches: impl Iterator<Item = T>) -> Option<T> {
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

impl Relationships {
    fn update_root_archive_count(&mut self, position: usize, archived: bool) {
        if self.parents[self.slots[position]].is_none() {
            if archived {
                self.archived_roots += 1;
            } else {
                self.archived_roots -= 1;
            }
        }
    }

    fn move_slot(&mut self, from: usize, to: usize, path: &Path) {
        let slot = self.slots[from];
        let parent = self.parents[slot];
        let child_rank =
            parent.map(|p| self.children[p].partition_point(|&s| self.positions[s] < from));
        let running_rank = self
            .running
            .binary_search_by_key(&from, |&s| self.positions[s])
            .ok();
        let start = from.min(to);
        let end = from.max(to);
        let changed = &mut self.slots[start..=end];
        if to < from {
            changed.rotate_right(1);
        } else {
            changed.rotate_left(1);
        }
        for (offset, &slot) in changed.iter().enumerate() {
            self.positions[slot] = start + offset;
        }
        if let Some(shared) = self.shared_paths.get_mut(path) {
            let last = self
                .by_path
                .get_mut(path)
                .expect("shared locator is indexed");
            shared.push(*last);
            shared.sort_unstable_by_key(|&s| self.positions[s]);
            *last = shared.pop().expect("previous owner");
        }
        let reorder = |ordered: &mut [usize], old: usize| {
            if to < from {
                let new = ordered[..old].partition_point(|&s| self.positions[s] < to);
                ordered[new..=old].rotate_right(1);
            } else {
                let crossed = ordered[old + 1..].partition_point(|&s| self.positions[s] < to);
                ordered[old..=old + crossed].rotate_left(1);
            }
        };
        if let (Some(parent), Some(rank)) = (parent, child_rank) {
            reorder(&mut self.children[parent], rank);
        }
        if let Some(rank) = running_rank {
            reorder(&mut self.running, rank);
        }
    }

    fn set_running(&mut self, slot: usize, running: bool) {
        let position = self.positions[slot];
        match self
            .running
            .binary_search_by_key(&position, |&s| self.positions[s])
        {
            Ok(rank) if !running => {
                self.running.remove(rank);
            }
            Err(rank) if running => self.running.insert(rank, slot),
            _ => {}
        }
    }

    fn indices_at_path(&self, path: &Path) -> impl Iterator<Item = usize> {
        self.by_path
            .get(path)
            .into_iter()
            .chain(self.shared_paths.get(path).into_iter().flatten())
            .copied()
    }

    fn row_index(&self, row: &SessionSummary, sessions: &[SessionSummary]) -> Option<usize> {
        if row.app_session_id > 0 {
            self.by_app_id.get(&row.app_session_id).copied()
        } else {
            self.indices_at_path(&row.path).find(|&i| {
                let candidate = &sessions[self.positions[i]];
                candidate.harness == row.harness
                    && candidate.profile_id == row.profile_id
                    && candidate.project == row.project
                    && candidate.id == row.id
            })
        }
    }
}

impl From<Vec<SessionSummary>> for SessionCatalog {
    fn from(sessions: Vec<SessionSummary>) -> Self {
        Self {
            sessions,
            relationships: OnceCell::new(),
            requested_archive_paths: HashSet::new(),
            #[cfg(any(test, feature = "test-support"))]
            relationship_builds: Default::default(),
        }
    }
}

impl Deref for SessionCatalog {
    type Target = Vec<SessionSummary>;

    fn deref(&self) -> &Self::Target {
        &self.sessions
    }
}

impl DerefMut for SessionCatalog {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.relationships.take();
        &mut self.sessions
    }
}

impl SessionCatalog {
    #[cfg(any(test, feature = "test-support"))]
    pub fn relationship_build_count(&self) -> usize {
        self.relationship_builds.get()
    }

    fn relationships(&self) -> &Relationships {
        self.relationships.get_or_init(|| {
            #[cfg(any(test, feature = "test-support"))]
            self.relationship_builds
                .set(self.relationship_builds.get() + 1);
            let index = SessionRootIndex::new(&self.sessions);
            let mut by_path = HashMap::new();
            let mut shared_paths = HashMap::<PathBuf, Vec<usize>>::new();
            for (i, row) in self.sessions.iter().enumerate() {
                if let Some(previous) = by_path.insert(row.path.clone(), i) {
                    shared_paths
                        .entry(row.path.clone())
                        .or_default()
                        .push(previous);
                }
            }
            let mut by_app_id = HashMap::new();
            let mut duplicate_app_ids = HashSet::new();
            let mut unbound = Vec::new();
            for (index, row) in self.sessions.iter().enumerate() {
                if row.app_session_id <= 0 {
                    unbound.push(UnboundRow {
                        slot: index,
                        canonical_path: OnceCell::new(),
                        canonical_project: OnceCell::new(),
                    });
                } else if by_app_id.insert(row.app_session_id, index).is_some() {
                    duplicate_app_ids.insert(row.app_session_id);
                }
            }
            let running = self
                .sessions
                .iter()
                .enumerate()
                .filter_map(|(i, row)| row.is_running.then_some(i))
                .collect();
            let parents: Vec<_> = self
                .sessions
                .iter()
                .map(|session| {
                    let parent = index.parent(session)?;
                    if let Some(&i) = by_app_id.get(&parent.app_session_id) {
                        return Some(i);
                    }
                    by_path
                        .get(&parent.path)
                        .into_iter()
                        .chain(shared_paths.get(&parent.path).into_iter().flatten())
                        .copied()
                        .find(|&i| std::ptr::eq(&self.sessions[i], parent))
                })
                .collect();
            let mut children = vec![Vec::new(); self.sessions.len()];
            let mut native_parents = HashSet::new();
            let mut archived_roots = 0;
            for (child, parent) in parents.iter().enumerate() {
                if let Some(parent) = parent {
                    children[*parent].push(child);
                    if self.sessions[child].parent_app_session_id.is_none() {
                        native_parents.insert(*parent);
                    }
                } else if self.sessions[child].archived {
                    archived_roots += 1;
                }
            }
            let native_worker_parents: HashMap<_, _> = native_parents
                .into_iter()
                .map(|index| (self.sessions[index].native_identity(), index))
                .collect();
            Relationships {
                slots: (0..self.sessions.len()).collect(),
                positions: (0..self.sessions.len()).collect(),
                by_path,
                shared_paths,
                by_app_id,
                duplicate_app_ids,
                unbound,
                running,
                parents,
                archived_roots,
                children,
                native_worker_parents,
            }
        })
    }

    fn row_at_slot(&self, slot: usize) -> &SessionSummary {
        &self.sessions[self.relationships().positions[slot]]
    }

    pub fn remember_archive_path(&mut self, original: &Path) {
        self.requested_archive_paths.insert(original.to_owned());
    }

    fn record_position(&self, row: &SessionSummary) -> Option<usize> {
        let cache = self.relationships();
        cache
            .by_app_id
            .get(&row.app_session_id)
            .copied()
            .or_else(|| {
                cache
                    .indices_at_path(&row.path)
                    .find(|&s| self.sessions[cache.positions[s]].same_record(row))
            })
            .or_else(|| {
                if row.app_session_id <= 0 {
                    return None;
                }
                let candidates = cache.unbound.iter().filter(|candidate| {
                    let legacy = &self.sessions[cache.positions[candidate.slot]];
                    legacy_identity_matches(legacy, row)
                        && (legacy.id == row.id
                            || self.requested_archive_paths.contains(&legacy.path))
                });
                let candidate = only_match(candidates.clone()).or_else(|| {
                    if self.requested_archive_paths.is_empty() {
                        return None;
                    }
                    only_match(candidates.filter(|candidate| {
                        let original = &self.sessions[cache.positions[candidate.slot]].path;
                        self.requested_archive_paths.contains(original)
                    }))
                })?;
                let legacy = &self.sessions[cache.positions[candidate.slot]];
                let same_path = legacy.path == row.path
                    || candidate
                        .canonical_path
                        .get_or_init(|| crate::normalize_session_path(&legacy.path))
                        == &row.path;
                (same_path
                    && (legacy.project == row.project
                        || candidate
                            .canonical_project
                            .get_or_init(|| crate::normalize_session_path(&legacy.project))
                            == &row.project))
                    .then_some(candidate.slot)
            })
            .map(|s| cache.positions[s])
    }

    pub fn matching_record(&self, row: &SessionSummary) -> Option<&SessionSummary> {
        self.record_position(row).map(|i| &self.sessions[i])
    }

    pub fn upsert_by_recency(&mut self, row: SessionSummary) {
        let previous = self.record_position(&row);
        let split = self
            .sessions
            .partition_point(|other| other.modified > row.modified);
        let to = split - usize::from(previous.is_some_and(|i| i < split));
        self.upsert_at(row, previous, to);
    }

    pub fn upsert_in_place(&mut self, row: SessionSummary) -> bool {
        let previous = self.record_position(&row);
        self.upsert_at(row, previous, previous.unwrap_or(self.sessions.len()))
    }

    fn upsert_at(&mut self, row: SessionSummary, previous: Option<usize>, to: usize) -> bool {
        let Some(from) = previous else {
            self.relationships.take();
            self.sessions.insert(to, row);
            return true;
        };
        let old = &self.sessions[from];
        if from == to && old == &row {
            return false;
        }
        let cache = self.relationships.get_mut().expect("indexed record lookup");
        let layout_changed = old.app_session_id != row.app_session_id
            || old.id != row.id
            || old.path != row.path
            || old.project != row.project
            || old.harness != row.harness
            || old.profile_id != row.profile_id
            || old.parent_app_session_id != row.parent_app_session_id
            || old.parent_session != row.parent_session
            || old.parent_harness != row.parent_harness;
        let range = from.min(to)..=from.max(to);
        let order_changes_lookup = from != to
            && (cache.duplicate_app_ids.contains(&row.app_session_id)
                || (!cache.native_worker_parents.is_empty()
                    && cache
                        .native_worker_parents
                        .contains_key(&row.native_identity())
                    && self.sessions[range.clone()].iter().any(|other| {
                        !std::ptr::eq(other, old)
                            && other.id == row.id
                            && other.harness == row.harness
                            && other.profile_id == row.profile_id
                            && other.project == row.project
                    })));
        if layout_changed || order_changes_lookup {
            if old.app_session_id <= 0 && row.app_session_id > 0 {
                self.requested_archive_paths.remove(&old.path);
            }
            self.relationships.take();
        } else {
            if old.archived != row.archived {
                cache.update_root_archive_count(from, row.archived);
            }
            if old.is_running != row.is_running {
                let slot = cache.slots[from];
                cache.set_running(slot, row.is_running);
            }
            if from != to {
                cache.move_slot(from, to, &row.path);
            }
        }
        self.sessions[from] = row;
        if to < from {
            self.sessions[range].rotate_right(1);
        } else if to > from {
            self.sessions[range].rotate_left(1);
        }
        true
    }

    pub fn native_worker_parent(
        &self,
        identity: &NativeSessionIdentity,
    ) -> Option<&SessionSummary> {
        self.relationships()
            .native_worker_parents
            .get(identity)
            .map(|&i| self.row_at_slot(i))
    }

    pub fn root_for_path(&self, selected: Option<&Path>) -> Option<&SessionSummary> {
        let selected = selected?;
        let relationships = self.relationships();
        let current = *relationships.by_path.get(selected)?;
        Some(self.root_at(current))
    }

    pub fn root_for_row(&self, row: &SessionSummary) -> Option<&SessionSummary> {
        self.relationships()
            .row_index(row, &self.sessions)
            .map(|index| self.root_at(index))
    }

    pub fn is_root_row(&self, row: &SessionSummary) -> bool {
        let links = self.relationships();
        links
            .row_index(row, &self.sessions)
            .is_some_and(|slot| links.parents[slot].is_none())
    }

    pub fn archived_root_count(&self) -> usize {
        self.relationships().archived_roots
    }

    fn root_at(&self, mut current: usize) -> &SessionSummary {
        let relationships = self.relationships();
        for _ in 0..self.sessions.len() {
            let Some(parent) = relationships.parents[current] else {
                break;
            };
            current = parent;
        }
        self.row_at_slot(current)
    }

    pub fn by_app_id(&self, id: i64) -> Option<&SessionSummary> {
        self.relationships()
            .by_app_id
            .get(&id)
            .map(|&index| self.row_at_slot(index))
    }

    pub fn by_path(&self, path: &Path) -> Option<&SessionSummary> {
        self.relationships()
            .by_path
            .get(path)
            .map(|&index| self.row_at_slot(index))
    }

    pub fn at_path(&self, path: &Path) -> impl Iterator<Item = &SessionSummary> {
        self.relationships()
            .indices_at_path(path)
            .map(|i| self.row_at_slot(i))
    }

    pub fn has_shared_path(&self, path: &Path) -> bool {
        self.relationships().shared_paths.contains_key(path)
    }

    pub fn by_key(&self, key: &crate::SessionKey) -> Option<&SessionSummary> {
        match key {
            crate::SessionKey::App(id) => self.by_app_id(id.get()),
            crate::SessionKey::Locator {
                harness,
                profile_id,
                path,
            } => self
                .at_path(path)
                .find(|row| row.harness == *harness && row.profile_id == *profile_id),
        }
    }

    fn archive_result_position(
        &mut self,
        stored: &SessionSummary,
        requested_path: Option<&Path>,
    ) -> Option<(usize, Option<crate::SessionTarget>)> {
        let cache = self.relationships();
        let saved = cache
            .by_app_id
            .get(&stored.app_session_id)
            .map(|&slot| cache.positions[slot]);
        let legacy_at = |path: &Path| {
            only_match(
                cache
                    .indices_at_path(path)
                    .map(|slot| cache.positions[slot])
                    .filter(|&i| {
                        let row = &self.sessions[i];
                        row.app_session_id <= 0
                            && legacy_identity_matches(row, stored)
                            && same_stored_project(row, stored)
                    }),
            )
        };
        let requested = requested_path.and_then(&legacy_at);
        if let Some(saved) = saved {
            let Some(old) = requested else {
                return Some((saved, None));
            };
            let target = self.sessions.remove(old).target();
            self.requested_archive_paths.remove(&target.path);
            self.relationships.take();
            return Some((saved - usize::from(old < saved), Some(target)));
        }
        let index = requested.or_else(|| legacy_at(&stored.path))?;
        if stored.app_session_id <= 0 {
            return Some((index, None));
        }
        self.sessions[index].app_session_id = stored.app_session_id;
        self.requested_archive_paths
            .remove(&self.sessions[index].path);
        self.relationships.take();
        Some((index, Some(self.sessions[index].target())))
    }

    pub fn adopt_archive_identity(
        &mut self,
        stored: &SessionSummary,
        requested_path: Option<&Path>,
    ) -> Option<crate::SessionTarget> {
        self.archive_result_position(stored, requested_path)?.1
    }

    pub fn apply_archive_result(
        &mut self,
        stored: &SessionSummary,
        requested_path: Option<&Path>,
        archived: bool,
    ) -> Option<usize> {
        let (position, _) = self.archive_result_position(stored, requested_path)?;
        self.set_archive_flag(position, archived);
        Some(position)
    }

    fn set_archive_flag(&mut self, position: usize, archived: bool) -> bool {
        if self.sessions[position].archived == archived {
            return false;
        }
        self.sessions[position].archived = archived;
        if let Some(cache) = self.relationships.get_mut() {
            cache.update_root_archive_count(position, archived);
        }
        true
    }

    pub fn set_row_archived(
        &mut self,
        native: &crate::SessionTarget,
        app_session_id: i64,
        archived: bool,
    ) -> bool {
        let cache = self.relationships();
        let index = cache.by_app_id.get(&app_session_id).copied().or_else(|| {
            cache.indices_at_path(&native.path).find(|&i| {
                let row = &self.sessions[cache.positions[i]];
                row.app_session_id <= 0 && row.harness == native.harness && row.id == native.id
            })
        });
        let Some(index) = index else { return false };
        let index = cache.positions[index];
        self.set_archive_flag(index, archived)
    }

    pub fn set_running(
        &mut self,
        key: &crate::SessionKey,
        running: bool,
    ) -> Option<&SessionSummary> {
        let row = self.by_key(key)?;
        let i = self.relationships().row_index(row, &self.sessions)?;
        self.set_slot_running(i, running)
    }

    pub fn set_record_running(
        &mut self,
        record: &crate::activity::SessionStatusRecord,
        running: bool,
    ) -> Option<&SessionSummary> {
        let row = record.resolve(self)?;
        let i = self.relationships().row_index(row, &self.sessions)?;
        self.set_slot_running(i, running)
    }

    fn set_slot_running(&mut self, i: usize, running: bool) -> Option<&SessionSummary> {
        let position = self.relationships().positions[i];
        if self.sessions[position].is_running == running {
            return None;
        }
        self.sessions[position].is_running = running;
        let cache = self.relationships.get_mut().expect("initialized above");
        cache.set_running(i, running);
        Some(&self.sessions[position])
    }

    pub fn mark_stopped(&mut self, path: &Path) {
        self.relationships();
        let cache = self.relationships.get_mut().expect("initialized above");
        cache.running.retain(|&i| {
            let row = &mut self.sessions[cache.positions[i]];
            if row.path == path {
                row.is_running = false;
                false
            } else {
                true
            }
        });
    }

    pub fn running(&self) -> impl Iterator<Item = &SessionSummary> {
        self.relationships()
            .running
            .iter()
            .map(|&i| self.row_at_slot(i))
    }

    pub fn roots(&self) -> impl Iterator<Item = &SessionSummary> {
        let cache = self.relationships();
        self.sessions
            .iter()
            .zip(&cache.slots)
            .filter_map(|(session, &slot)| cache.parents[slot].is_none().then_some(session))
    }

    pub fn ancestors(&self, session: &SessionSummary) -> Vec<&SessionSummary> {
        let relationships = self.relationships();
        let Some(start) = relationships.row_index(session, &self.sessions) else {
            return Vec::new();
        };
        let mut seen = HashSet::from([start]);
        let mut ancestors = Vec::new();
        let mut current = relationships.parents[start];
        while let Some(parent) = current {
            if !seen.insert(parent) {
                break;
            }
            ancestors.push(self.row_at_slot(parent));
            current = relationships.parents[parent];
        }
        ancestors
    }

    pub fn active_worker_profile(&self, app_session_id: i64) -> Option<&str> {
        let child = self.by_app_id(app_session_id)?;
        let profile = child.worker_profile.as_deref()?;
        let parent = self.by_app_id(child.parent_app_session_id?)?;
        (!child.archived && !parent.archived).then_some(profile)
    }

    pub fn child_count(&self, session: &SessionSummary) -> usize {
        let relationships = self.relationships();
        relationships
            .row_index(session, &self.sessions)
            .map_or(0, |index| relationships.children[index].len())
    }

    pub fn descendants(&self, root: &SessionSummary) -> Vec<(&SessionSummary, usize)> {
        let relationships = self.relationships();
        let Some(root) = relationships.row_index(root, &self.sessions) else {
            return Vec::new();
        };
        let mut stack: Vec<_> = relationships.children[root]
            .iter()
            .rev()
            .map(|&child| (child, 1_usize))
            .collect();
        let mut seen = HashSet::from([root]);
        let mut descendants = Vec::new();
        while let Some((child, depth)) = stack.pop() {
            if !seen.insert(child) {
                continue;
            }
            descendants.push((self.row_at_slot(child), depth));
            stack.extend(
                relationships.children[child]
                    .iter()
                    .rev()
                    .map(|&child| (child, depth.saturating_add(1))),
            );
        }
        descendants
    }
}

#[cfg(test)]
#[path = "catalog_cache_tests.rs"]
mod tests;
