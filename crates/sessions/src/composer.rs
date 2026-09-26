use std::{collections::HashMap, ops::Range, path::Path};

const MAX_HISTORY: usize = 100;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposerRecord<A> {
    pub target: String,
    pub text: String,
    pub cursor: usize,
    pub selection_start: usize,
    pub selection_end: usize,
    pub history: Vec<String>,
    pub attachments: Vec<A>,
}

impl<A> Default for ComposerRecord<A> {
    fn default() -> Self {
        Self {
            target: String::new(),
            text: String::new(),
            cursor: 0,
            selection_start: 0,
            selection_end: 0,
            history: Vec::new(),
            attachments: Vec::new(),
        }
    }
}

pub trait ComposerPersistence<A> {
    fn save(&self, record: ComposerRecord<A>);
    fn delete(&self, target: String);
}

#[cfg(any(test, feature = "test-support"))]
struct NoopPersistence;

#[cfg(any(test, feature = "test-support"))]
impl<A> ComposerPersistence<A> for NoopPersistence {
    fn save(&self, _record: ComposerRecord<A>) {}
    fn delete(&self, _target: String) {}
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ComposerSnapshot {
    pub text: String,
    pub cursor: usize,
    pub selection: Range<usize>,
}

impl ComposerSnapshot {
    pub fn new(text: String, cursor: usize, selection: Range<usize>) -> Self {
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

    pub fn restore_range(&self) -> Range<usize> {
        if !self.selection.is_empty() && self.cursor == self.selection.start {
            self.selection.end..self.selection.start
        } else if self.selection.is_empty() {
            self.cursor..self.cursor
        } else {
            self.selection.clone()
        }
    }
}

struct SessionComposer<A> {
    composer: ComposerSnapshot,
    attachments: Vec<A>,
    history: Vec<String>,
    history_index: Option<usize>,
    history_draft: Option<ComposerSnapshot>,
}

impl<A> Default for SessionComposer<A> {
    fn default() -> Self {
        Self {
            composer: ComposerSnapshot::default(),
            attachments: Vec::new(),
            history: Vec::new(),
            history_index: None,
            history_draft: None,
        }
    }
}

impl<A: Clone + Eq> SessionComposer<A> {
    fn from_record(record: ComposerRecord<A>) -> Self {
        Self {
            composer: ComposerSnapshot::new(
                record.text,
                record.cursor,
                record.selection_start..record.selection_end,
            ),
            history: record.history,
            attachments: record.attachments,
            history_index: None,
            history_draft: None,
        }
    }

    fn record(&self, target: String) -> ComposerRecord<A> {
        ComposerRecord {
            target,
            text: self.composer.text.clone(),
            cursor: self.composer.cursor,
            selection_start: self.composer.selection.start,
            selection_end: self.composer.selection.end,
            history: self.history.clone(),
            attachments: self.attachments.clone(),
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

pub struct ComposerSessions<A> {
    current_target: String,
    sessions: HashMap<String, SessionComposer<A>>,
    persistence: Box<dyn ComposerPersistence<A>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HistoryNavigation {
    PassThrough,
    Handled(Option<ComposerSnapshot>),
}

impl<A: Clone + Eq> ComposerSessions<A> {
    pub fn new(
        current_target: String,
        records: Vec<ComposerRecord<A>>,
        persistence: Box<dyn ComposerPersistence<A>>,
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(current_target: String) -> Self {
        Self::new(current_target, Vec::new(), Box::new(NoopPersistence))
    }

    pub fn current(&self) -> ComposerSnapshot {
        self.sessions
            .get(&self.current_target)
            .map(|session| session.composer.clone())
            .unwrap_or_default()
    }

    pub fn current_target(&self) -> &str {
        &self.current_target
    }

    pub fn saved_attachments(&self) -> impl Iterator<Item = (&String, &[A])> {
        self.sessions
            .iter()
            .filter(|(_, session)| !session.attachments.is_empty())
            .map(|(target, session)| (target, session.attachments.as_slice()))
    }

    pub fn set_attachments(&mut self, target: &str, attachments: Vec<A>) {
        let session = self.sessions.entry(target.to_owned()).or_default();
        if session.attachments != attachments {
            session.attachments = attachments;
            self.persistence.save(session.record(target.to_owned()));
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn snapshot_for(&self, target: &str) -> ComposerSnapshot {
        self.sessions
            .get(target)
            .map(|session| session.composer.clone())
            .unwrap_or_default()
    }

    pub fn capture_current(&mut self, snapshot: ComposerSnapshot) {
        let target = self.current_target.clone();
        let session = self.sessions.entry(target.clone()).or_default();
        if session.composer == snapshot {
            return;
        }
        session.composer = snapshot;
        self.persistence.save(session.record(target));
    }

    pub fn switch_to(&mut self, target: String, current: ComposerSnapshot) -> ComposerSnapshot {
        self.capture_current(current);
        if let Some(session) = self.sessions.get_mut(&self.current_target) {
            session.history_index = None;
            session.history_draft = None;
        }
        self.current_target = target;
        self.current()
    }

    pub fn discard_and_switch(&mut self, target: &str, next: String) -> ComposerSnapshot {
        self.sessions.remove(target);
        self.persistence.delete(target.to_owned());
        self.current_target = next;
        self.current()
    }

    pub fn remove(&mut self, target: &str) {
        self.sessions.remove(target);
        self.persistence.delete(target.to_owned());
    }

    pub fn promote(&mut self, from: &str, to: String) {
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
        target.attachments.extend(source.attachments);
        if self.current_target == from {
            self.current_target = to.clone();
        }
        self.persistence.delete(from.to_owned());
        self.persistence.save(target.record(to));
    }

    pub fn record_submission(&mut self, target: &str, text: &str) {
        let session = self.sessions.entry(target.to_owned()).or_default();
        let changed = session.add_history(text);
        session.history_index = None;
        session.history_draft = None;
        if changed {
            self.persistence.save(session.record(target.to_owned()));
        }
    }

    pub fn clear_submitted_text(&mut self, target: &str, text: &str) -> bool {
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

    pub fn restore_submitted_text(
        &mut self,
        target: &str,
        text: String,
    ) -> Option<ComposerSnapshot> {
        let session = self.sessions.entry(target.to_owned()).or_default();
        if !session.composer.text.is_empty() {
            return None;
        }
        let cursor = text.len();
        session.composer = ComposerSnapshot::new(text, cursor, cursor..cursor);
        self.persistence.save(session.record(target.to_owned()));
        Some(session.composer.clone())
    }

    pub fn append_to_draft(&mut self, target: &str, text: &str) -> ComposerSnapshot {
        let session = self.sessions.entry(target.to_owned()).or_default();
        if !session.composer.text.is_empty() {
            session.composer.text.push_str("\n\n");
        }
        session.composer.text.push_str(text);
        session.history_index = None;
        session.history_draft = None;
        self.persistence.save(session.record(target.to_owned()));
        session.composer.clone()
    }

    pub fn sync_history(&mut self, target: &str, messages: &[String]) {
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

    pub fn exit_history(&mut self) {
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

    pub fn navigate_history(&mut self, key: &str, current: ComposerSnapshot) -> HistoryNavigation {
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

    pub fn previous_history(&mut self, current: ComposerSnapshot) -> Option<ComposerSnapshot> {
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

pub fn draft_target(id: &str) -> String {
    format!("draft:{id}")
}

pub fn session_target(path: &Path) -> String {
    format!("session:{}", path.display())
}

pub fn project_target(path: &Path) -> String {
    format!("project:{}", path.display())
}

pub fn draft_id(target: &str) -> Option<&str> {
    target.strip_prefix("draft:")
}

pub fn session_path(target: &str) -> Option<&Path> {
    target.strip_prefix("session:").map(Path::new)
}

#[cfg(test)]
#[path = "composer_tests.rs"]
mod tests;
