use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const NOTICE_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_PROJECT_NOTICES: usize = 256;

#[derive(Clone)]
pub(crate) struct NoticeBoard {
    entries: Arc<Mutex<HashMap<PathBuf, Vec<Notice>>>>,
    updates: async_channel::Sender<()>,
    update_receiver: async_channel::Receiver<()>,
}

#[derive(Clone)]
struct Notice {
    from_id: String,
    from_name: String,
    message: String,
    paths: Vec<PathBuf>,
    created_at: Instant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NoticeView {
    pub(crate) from: String,
    pub(crate) message: String,
    pub(crate) paths: Vec<String>,
    pub(crate) age_seconds: u64,
}

impl NoticeBoard {
    pub(crate) fn updates(&self) -> async_channel::Receiver<()> {
        self.update_receiver.clone()
    }

    pub(crate) fn post(
        &self,
        project: &Path,
        from_id: String,
        from_name: String,
        message: String,
        paths: Vec<PathBuf>,
    ) -> Result<(), String> {
        let now = Instant::now();
        {
            let mut boards = self
                .entries
                .lock()
                .map_err(|_| "worker notice board is unavailable".to_owned())?;
            let board = boards.entry(project.to_owned()).or_default();
            prune(board, now);
            board.push(Notice {
                from_id,
                from_name,
                message,
                paths,
                created_at: now,
            });
            let excess = board.len().saturating_sub(MAX_PROJECT_NOTICES);
            board.drain(..excess);
        }
        let _ = self.updates.try_send(());
        Ok(())
    }

    pub(crate) fn matching(
        &self,
        project: &Path,
        excluded_worker: &str,
        paths: &[PathBuf],
    ) -> Result<Vec<NoticeView>, String> {
        self.read(project, |board, now| {
            board
                .iter()
                .filter(|notice| notice.from_id != excluded_worker)
                .filter(|notice| relevant(&notice.paths, paths))
                .map(|notice| notice_view(notice, now))
                .collect()
        })
    }

    pub(crate) fn snapshot(&self, project: &Path) -> Vec<NoticeView> {
        self.read(project, |board, now| {
            board
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
        read: impl FnOnce(&[Notice], Instant) -> T,
    ) -> Result<T, String> {
        let now = Instant::now();
        let mut boards = self
            .entries
            .lock()
            .map_err(|_| "worker notice board is unavailable".to_owned())?;
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
        }
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

fn prune(board: &mut Vec<Notice>, now: Instant) {
    board.retain(|notice| now.duration_since(notice.created_at) < NOTICE_TTL);
}

#[cfg(test)]
#[path = "worker_notices_tests.rs"]
mod tests;
