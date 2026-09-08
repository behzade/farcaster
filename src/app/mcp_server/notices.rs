use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use path_clean::PathClean as _;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::agents::CallerContext;

const NOTICE_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_PROJECT_NOTICES: usize = 256;
const MAX_MESSAGE_BYTES: usize = 2_000;
const MAX_PATHS: usize = 64;
const MAX_PATH_BYTES: usize = 1_024;

#[derive(Clone, Default)]
pub(super) struct NoticeBoard {
    entries: Arc<Mutex<HashMap<PathBuf, Vec<Notice>>>>,
}

#[derive(Clone)]
struct Notice {
    from_id: String,
    from_name: String,
    message: String,
    paths: Vec<PathBuf>,
    created_at: Instant,
}

#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum Action {
    Read,
    Post,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(super) struct Params {
    pub(super) action: Action,
    pub(super) message: Option<String>,
    #[serde(default)]
    pub(super) paths: Vec<String>,
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct Response {
    posted: bool,
    notices: Vec<NoticeView>,
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct NoticeView {
    from: String,
    message: String,
    paths: Vec<String>,
    age_seconds: u64,
}

impl NoticeBoard {
    pub(super) fn access(
        &self,
        caller: &CallerContext,
        params: Params,
    ) -> Result<Response, String> {
        if caller.parent_worker_id.is_some() {
            return Err("worker notices are available only to top-level workers".into());
        }
        let action = params.action;
        let paths = normalize_paths(params.paths)?;
        let message = match (action, params.message) {
            (Action::Read, None) => None,
            (Action::Read, Some(_)) => {
                return Err("worker notice `message` is valid only when action is `post`".into());
            }
            (Action::Post, Some(message)) if !message.trim().is_empty() => {
                if message.len() > MAX_MESSAGE_BYTES {
                    return Err(format!(
                        "worker notice must be at most {MAX_MESSAGE_BYTES} bytes"
                    ));
                }
                Some(message.trim().to_owned())
            }
            (Action::Post, _) => return Err("worker notice posts require `message`".into()),
        };

        let now = Instant::now();
        let mut boards = self
            .entries
            .lock()
            .map_err(|_| "worker notice board is unavailable".to_owned())?;
        let board = boards.entry(caller.project.clone()).or_default();
        prune(board, now);
        if let Some(message) = message {
            board.push(Notice {
                from_id: caller.worker_id.clone(),
                from_name: caller.worker_name.clone(),
                message,
                paths: paths.clone(),
                created_at: now,
            });
            let excess = board.len().saturating_sub(MAX_PROJECT_NOTICES);
            board.drain(..excess);
        }
        let notices = board
            .iter()
            .filter(|notice| notice.from_id != caller.worker_id)
            .filter(|notice| relevant(&notice.paths, &paths))
            .map(|notice| NoticeView {
                from: notice.from_name.clone(),
                message: notice.message.clone(),
                paths: notice
                    .paths
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect(),
                age_seconds: now.duration_since(notice.created_at).as_secs(),
            })
            .collect();
        Ok(Response {
            posted: matches!(action, Action::Post),
            notices,
        })
    }
}

fn normalize_paths(paths: Vec<String>) -> Result<Vec<PathBuf>, String> {
    if paths.len() > MAX_PATHS {
        return Err(format!("worker notice accepts at most {MAX_PATHS} paths"));
    }
    let mut normalized = Vec::with_capacity(paths.len());
    for path in paths {
        let path = path.trim();
        if path.is_empty() || path.len() > MAX_PATH_BYTES {
            return Err(format!(
                "worker notice paths must be 1-{MAX_PATH_BYTES} bytes"
            ));
        }
        let path = Path::new(path).clean();
        if path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
        {
            return Err("worker notice paths must stay within the project".into());
        }
        if !normalized.contains(&path) {
            normalized.push(path);
        }
    }
    Ok(normalized)
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
#[path = "notices_tests.rs"]
mod tests;
