//! Composer text, selection, and prompt history keyed by session.

use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::Path,
    sync::mpsc::{self, RecvTimeoutError, Sender},
    thread::JoinHandle,
    time::Duration,
};

use crate::state::{ComposerRecord, StateStore};

const MAX_HISTORY: usize = 100;
const WRITE_DELAY: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ComposerSnapshot {
    pub text: String,
    pub cursor: usize,
    pub selection: Range<usize>,
}

impl ComposerSnapshot {
    pub(crate) fn new(text: String, cursor: usize, selection: Range<usize>) -> Self {
        let len = text.len();
        let mut selection = selection.start.min(len)..selection.end.min(len);
        if selection.start > selection.end {
            selection = selection.end..selection.start;
        }
        Self {
            text,
            cursor: cursor.min(len),
            selection,
        }
    }

    pub(crate) fn restore_range(&self) -> Range<usize> {
        if !self.selection.is_empty() && self.cursor == self.selection.start {
            self.selection.end..self.selection.start
        } else if self.selection.is_empty() {
            self.cursor..self.cursor
        } else {
            self.selection.clone()
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct SessionComposer {
    composer: ComposerSnapshot,
    history: Vec<String>,
    history_index: Option<usize>,
    history_draft: Option<ComposerSnapshot>,
}

impl SessionComposer {
    fn from_record(record: ComposerRecord) -> Self {
        Self {
            composer: ComposerSnapshot::new(
                record.text,
                record.cursor,
                record.selection_start..record.selection_end,
            ),
            history: record.history,
            history_index: None,
            history_draft: None,
        }
    }

    fn record(&self, target: String) -> ComposerRecord {
        ComposerRecord {
            target,
            text: self.composer.text.clone(),
            cursor: self.composer.cursor,
            selection_start: self.composer.selection.start,
            selection_end: self.composer.selection.end,
            history: self.history.clone(),
        }
    }

    fn add_history(&mut self, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() || self.history.first().is_some_and(|entry| entry == text) {
            return false;
        }
        self.history.insert(0, text.to_owned());
        self.history.truncate(MAX_HISTORY);
        true
    }
}

pub(crate) struct ComposerSessions {
    current_target: String,
    sessions: HashMap<String, SessionComposer>,
    persistence: ComposerPersistence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HistoryNavigation {
    PassThrough,
    Handled(Option<ComposerSnapshot>),
}

impl ComposerSessions {
    pub(crate) fn load(current_target: String) -> (Self, Option<String>) {
        let (records, error) =
            match StateStore::open().and_then(|store| store.load_composer_sessions()) {
                Ok(records) => (records, None),
                Err(error) => (Vec::new(), Some(error)),
            };
        (
            Self::from_records(current_target, records, ComposerPersistence::spawn()),
            error,
        )
    }

    fn from_records(
        current_target: String,
        records: Vec<ComposerRecord>,
        persistence: ComposerPersistence,
    ) -> Self {
        let sessions = records
            .into_iter()
            .map(|record| (record.target.clone(), SessionComposer::from_record(record)))
            .collect();
        Self {
            current_target,
            sessions,
            persistence,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(current_target: String) -> Self {
        Self::from_records(current_target, Vec::new(), ComposerPersistence::noop())
    }

    pub(crate) fn current(&self) -> ComposerSnapshot {
        self.sessions
            .get(&self.current_target)
            .map(|session| session.composer.clone())
            .unwrap_or_default()
    }

    pub(crate) fn current_target(&self) -> &str {
        &self.current_target
    }

    #[cfg(test)]
    pub(crate) fn snapshot_for(&self, target: &str) -> ComposerSnapshot {
        self.sessions
            .get(target)
            .map(|session| session.composer.clone())
            .unwrap_or_default()
    }

    pub(crate) fn capture_current(&mut self, snapshot: ComposerSnapshot) {
        let target = self.current_target.clone();
        let session = self.sessions.entry(target.clone()).or_default();
        if session.composer == snapshot {
            return;
        }
        session.composer = snapshot;
        self.persistence.save(session.record(target));
    }

    pub(crate) fn switch_to(
        &mut self,
        target: String,
        current: ComposerSnapshot,
    ) -> ComposerSnapshot {
        self.capture_current(current);
        if let Some(session) = self.sessions.get_mut(&self.current_target) {
            session.history_index = None;
            session.history_draft = None;
        }
        self.current_target = target;
        self.current()
    }

    pub(crate) fn discard_and_switch(&mut self, target: &str, next: String) -> ComposerSnapshot {
        self.sessions.remove(target);
        self.persistence.delete(target.to_owned());
        self.current_target = next;
        self.current()
    }

    pub(crate) fn promote(&mut self, from: &str, to: String) {
        let Some(mut source) = self.sessions.remove(from) else {
            if self.current_target == from {
                self.current_target = to;
            }
            self.persistence.delete(from.to_owned());
            return;
        };
        source.history_index = None;
        source.history_draft = None;
        let target = self.sessions.entry(to.clone()).or_default();
        if !source.composer.text.is_empty() || target.composer.text.is_empty() {
            target.composer = source.composer;
        }
        if !source.history.is_empty() {
            target.history = source.history;
        }
        if self.current_target == from {
            self.current_target = to.clone();
        }
        self.persistence.delete(from.to_owned());
        self.persistence.save(target.record(to));
    }

    pub(crate) fn record_submission(&mut self, target: &str, text: &str) {
        let session = self.sessions.entry(target.to_owned()).or_default();
        let changed = session.add_history(text);
        session.history_index = None;
        session.history_draft = None;
        if changed {
            self.persistence.save(session.record(target.to_owned()));
        }
    }

    pub(crate) fn clear_submitted_text(&mut self, target: &str, text: &str) -> bool {
        let Some(session) = self.sessions.get_mut(target) else {
            return false;
        };
        if session.composer.text != text {
            return false;
        }
        session.composer = ComposerSnapshot::default();
        session.history_index = None;
        session.history_draft = None;
        self.persistence.save(session.record(target.to_owned()));
        true
    }

    pub(crate) fn sync_history(&mut self, target: &str, messages: &[String]) {
        if messages.is_empty() {
            return;
        }
        let mut history = Vec::new();
        for message in messages {
            let message = message.trim();
            if !message.is_empty() && history.last().is_none_or(|entry| entry != message) {
                history.push(message.to_owned());
            }
        }
        history.reverse();
        history.truncate(MAX_HISTORY);
        let session = self.sessions.entry(target.to_owned()).or_default();
        if session.history == history {
            return;
        }
        session.history = history;
        session.history_index = None;
        session.history_draft = None;
        self.persistence.save(session.record(target.to_owned()));
    }

    pub(crate) fn exit_history(&mut self) {
        if let Some(session) = self.sessions.get_mut(&self.current_target) {
            session.history_index = None;
            session.history_draft = None;
        }
    }

    fn is_browsing_history(&self) -> bool {
        self.sessions
            .get(&self.current_target)
            .is_some_and(|session| session.history_index.is_some())
    }

    pub(crate) fn navigate_history(
        &mut self,
        key: &str,
        current: ComposerSnapshot,
    ) -> HistoryNavigation {
        let before_cursor = current.text.get(..current.cursor).unwrap_or_default();
        let after_cursor = current.text.get(current.cursor..).unwrap_or_default();
        let browsing = self.is_browsing_history();
        match key {
            "up" if !before_cursor.contains('\n') => {
                HistoryNavigation::Handled(self.previous_history(current))
            }
            "down" if browsing && !after_cursor.contains('\n') => {
                HistoryNavigation::Handled(self.next_history())
            }
            _ => HistoryNavigation::PassThrough,
        }
    }

    pub(crate) fn previous_history(
        &mut self,
        current: ComposerSnapshot,
    ) -> Option<ComposerSnapshot> {
        let target = self.current_target.clone();
        let session = self.sessions.entry(target.clone()).or_default();
        let next = session
            .history_index
            .map_or(0, |index| index.saturating_add(1));
        if next >= session.history.len() {
            return None;
        }
        if session.history_index.is_none() {
            session.history_draft = Some(current);
        }
        session.history_index = Some(next);
        let text = session.history[next].clone();
        session.composer = ComposerSnapshot::new(text, 0, 0..0);
        self.persistence.save(session.record(target));
        Some(session.composer.clone())
    }

    fn next_history(&mut self) -> Option<ComposerSnapshot> {
        let target = self.current_target.clone();
        let session = self.sessions.get_mut(&target)?;
        let index = session.history_index?;
        session.composer = if index == 0 {
            session.history_index = None;
            session.history_draft.take().unwrap_or_default()
        } else {
            let next = index - 1;
            session.history_index = Some(next);
            let text = session.history[next].clone();
            let cursor = text.len();
            ComposerSnapshot::new(text, cursor, cursor..cursor)
        };
        self.persistence.save(session.record(target));
        Some(session.composer.clone())
    }
}

pub(crate) fn draft_target(id: &str) -> String {
    format!("draft:{id}")
}

pub(crate) fn session_target(path: &Path) -> String {
    format!("session:{}", path.display())
}

pub(crate) fn project_target(path: &Path) -> String {
    format!("project:{}", path.display())
}

enum PersistenceCommand {
    Save(ComposerRecord),
    Delete(String),
    Shutdown,
}

struct ComposerPersistence {
    sender: Sender<PersistenceCommand>,
    worker: Option<JoinHandle<()>>,
}

impl ComposerPersistence {
    fn spawn() -> Self {
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("pi-gpui-composer-state".into())
            .spawn(move || {
                let Ok(store) = StateStore::open() else {
                    while !matches!(receiver.recv(), Ok(PersistenceCommand::Shutdown) | Err(_)) {}
                    return;
                };
                let mut pending = HashMap::new();
                let mut deleted = HashSet::new();
                loop {
                    match receiver.recv_timeout(WRITE_DELAY) {
                        Ok(PersistenceCommand::Save(record)) => {
                            deleted.remove(&record.target);
                            pending.insert(record.target.clone(), record);
                        }
                        Ok(PersistenceCommand::Delete(target)) => {
                            pending.remove(&target);
                            deleted.insert(target);
                        }
                        Ok(PersistenceCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                            flush(&store, &mut pending, &mut deleted);
                            break;
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            flush(&store, &mut pending, &mut deleted);
                        }
                    }
                }
            })
            .ok();
        Self { sender, worker }
    }

    fn save(&self, record: ComposerRecord) {
        let _ = self.sender.send(PersistenceCommand::Save(record));
    }

    fn delete(&self, target: String) {
        let _ = self.sender.send(PersistenceCommand::Delete(target));
    }

    #[cfg(test)]
    fn noop() -> Self {
        let (sender, receiver) = mpsc::channel();
        drop(receiver);
        Self {
            sender,
            worker: None,
        }
    }
}

impl Drop for ComposerPersistence {
    fn drop(&mut self) {
        let _ = self.sender.send(PersistenceCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn flush(
    store: &StateStore,
    pending: &mut HashMap<String, ComposerRecord>,
    deleted: &mut HashSet<String>,
) {
    for target in deleted.drain() {
        let _ = store.delete_composer_session(&target);
    }
    for (_, record) in pending.drain() {
        let _ = store.save_composer_session(&record);
    }
}
