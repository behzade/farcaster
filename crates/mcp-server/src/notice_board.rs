use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const NOTICE_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_PROJECT_NOTICES: usize = 256;

#[derive(Clone)]
pub struct NoticeBoard {
    entries: Arc<Mutex<HashMap<PathBuf, ProjectNotices>>>,
    updates: async_channel::Sender<()>,
    update_receiver: async_channel::Receiver<()>,
    changes: tokio::sync::watch::Sender<()>,
}

struct ProjectNotices {
    generation: uuid::Uuid,
    sequence: u64,
    notices: Vec<Notice>,
}

pub(super) struct NoticeBatch {
    pub cursor: String,
    pub notices: Vec<NoticeView>,
}

#[derive(Clone)]
struct Notice {
    sequence: u64,
    from_id: String,
    from_name: String,
    message: String,
    paths: Vec<PathBuf>,
    created_at: Instant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoticeView {
    pub from: String,
    pub message: String,
    pub paths: Vec<String>,
    pub age_seconds: u64,
}

impl NoticeBoard {
    pub fn updates(&self) -> async_channel::Receiver<()> {
        self.update_receiver.clone()
    }

    pub fn post(
        &self,
        project: &Path,
        from_id: String,
        from_name: String,
        message: String,
        paths: Vec<PathBuf>,
    ) -> Result<(), String> {
        {
            let mut boards = self
                .entries
                .lock()
                .map_err(|_| "worker notice board is unavailable".to_owned())?;
            let now = Instant::now();
            let board = boards.entry(project.to_owned()).or_default();
            prune(board, now);
            board.sequence += 1;
            board.notices.push(Notice {
                sequence: board.sequence,
                from_id,
                from_name,
                message,
                paths,
                created_at: now,
            });
            let excess = board.notices.len().saturating_sub(MAX_PROJECT_NOTICES);
            board.notices.drain(..excess);
        }
        let _ = self.updates.try_send(());
        self.changes.send_replace(());
        Ok(())
    }

    pub(super) fn matching(
        &self,
        project: &Path,
        excluded_worker: &str,
        paths: &[PathBuf],
        after: Option<&str>,
    ) -> Result<NoticeBatch, String> {
        self.read(project, |board, now| {
            let after = after
                .map(|cursor| board.validate_cursor(cursor))
                .transpose()?;
            let notices = board
                .notices
                .iter()
                .filter(|notice| after.is_none_or(|after| notice.sequence > after))
                .filter(|notice| notice.from_id != excluded_worker)
                .filter(|notice| relevant(&notice.paths, paths))
                .map(|notice| notice_view(notice, now))
                .collect();
            Ok(NoticeBatch {
                cursor: format!("{}:{}", board.generation, board.sequence),
                notices,
            })
        })?
    }

    pub(super) async fn wait(
        &self,
        project: &Path,
        excluded_worker: &str,
        paths: &[PathBuf],
        after: &str,
        timeout: Duration,
    ) -> Result<NoticeBatch, String> {
        let mut changes = self.changes.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let batch = self.matching(project, excluded_worker, paths, Some(after))?;
            if !batch.notices.is_empty() || tokio::time::Instant::now() >= deadline {
                return Ok(batch);
            }
            let _ = tokio::time::timeout_at(deadline, changes.changed()).await;
        }
    }

    pub fn snapshot(&self, project: &Path) -> Vec<NoticeView> {
        self.read(project, |board, now| {
            board
                .notices
                .iter()
                .rev()
                .map(|notice| notice_view(notice, now))
                .collect()
        })
        .unwrap_or_default()
    }

    fn read<T>(
        &self,
        project: &Path,
        read: impl FnOnce(&ProjectNotices, Instant) -> T,
    ) -> Result<T, String> {
        let mut boards = self
            .entries
            .lock()
            .map_err(|_| "worker notice board is unavailable".to_owned())?;
        let now = Instant::now();
        let board = boards.entry(project.to_owned()).or_default();
        prune(board, now);
        Ok(read(board, now))
    }
}

impl Default for NoticeBoard {
    fn default() -> Self {
        let (updates, update_receiver) = async_channel::bounded(1);
        Self {
            entries: Arc::default(),
            updates,
            update_receiver,
            changes: tokio::sync::watch::channel(()).0,
        }
    }
}

impl Default for ProjectNotices {
    fn default() -> Self {
        Self {
            generation: uuid::Uuid::new_v4(),
            sequence: 0,
            notices: Vec::new(),
        }
    }
}

impl ProjectNotices {
    fn validate_cursor(&self, cursor: &str) -> Result<u64, String> {
        let oldest_cursor = self
            .notices
            .first()
            .map_or(self.sequence, |notice| notice.sequence - 1);
        let sequence = cursor
            .split_once(':')
            .filter(|(generation, _)| *generation == self.generation.to_string())
            .and_then(|(_, sequence)| sequence.parse::<u64>().ok());
        sequence
            .filter(|sequence| *sequence >= oldest_cursor && *sequence <= self.sequence)
            .ok_or_else(|| {
                "worker notice cursor is invalid or expired; read notices again for a fresh cursor"
                    .into()
            })
    }
}

fn notice_view(notice: &Notice, now: Instant) -> NoticeView {
    NoticeView {
        from: notice.from_name.clone(),
        message: notice.message.clone(),
        paths: notice
            .paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        age_seconds: now.duration_since(notice.created_at).as_secs(),
    }
}

fn relevant(notice: &[PathBuf], filter: &[PathBuf]) -> bool {
    filter.is_empty()
        || notice.is_empty()
        || notice.iter().any(|left| {
            filter
                .iter()
                .any(|right| left.starts_with(right) || right.starts_with(left))
        })
}

fn prune(board: &mut ProjectNotices, now: Instant) {
    board
        .notices
        .retain(|notice| now.duration_since(notice.created_at) < NOTICE_TTL);
}

#[cfg(test)]
#[path = "notice_board_tests.rs"]
mod tests;
