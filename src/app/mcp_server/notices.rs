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
    /// Read the board or post a notice and then read matching entries.
    pub(super) action: Action,
    /// Short coordination notice. Required only when action is `post`.
    pub(super) message: Option<String>,
    /// Optional project-relative files or directories used for relevance filtering.
    #[serde(default)]
    pub(super) paths: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    posted: bool,
    notices: Vec<NoticeView>,
}

#[derive(Serialize)]
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
    ) -> Result<serde_json::Value, String> {
        if caller.parent_worker_id.is_some() {
            return Err("worker notices are available only to top-level workers".into());
        }
        let now = Instant::now();
        let paths = normalize_paths(params.paths)?;
        let (posted, filter) = match params.action {
            Action::Read => {
                if params.message.is_some() {
                    return Err(
                        "worker notice `message` is valid only when action is `post`".into(),
                    );
                }
                (false, paths)
            }
            Action::Post => {
                let message = params
                    .message
                    .as_deref()
                    .ok_or_else(|| "worker notice posts require `message`".to_owned())?
                    .trim();
                if message.is_empty() {
                    return Err("worker notice must not be empty".into());
                }
                if message.len() > MAX_MESSAGE_BYTES {
                    return Err(format!(
                        "worker notice must be at most {MAX_MESSAGE_BYTES} bytes"
                    ));
                }
                let mut boards = self
                    .entries
                    .lock()
                    .map_err(|_| "worker notice board is unavailable".to_owned())?;
                let board = boards.entry(caller.project.clone()).or_default();
                prune(board, now);
                board.push(Notice {
                    from_id: caller.worker_id.clone(),
                    from_name: caller.worker_name.clone(),
                    message: message.to_owned(),
                    paths: paths.clone(),
                    created_at: now,
                });
                if board.len() > MAX_PROJECT_NOTICES {
                    board.drain(..board.len().saturating_sub(MAX_PROJECT_NOTICES));
                }
                (true, paths)
            }
        };

        let mut boards = self
            .entries
            .lock()
            .map_err(|_| "worker notice board is unavailable".to_owned())?;
        let board = boards.entry(caller.project.clone()).or_default();
        prune(board, now);
        let notices = board
            .iter()
            .filter(|notice| notice.from_id != caller.worker_id)
            .filter(|notice| relevant(&notice.paths, &filter))
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
        serde_json::to_value(Response { posted, notices })
            .map_err(|error| format!("serialize worker notices: {error}"))
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
mod tests {
    use super::*;

    fn caller(id: &str, name: &str) -> CallerContext {
        CallerContext {
            worker_id: id.into(),
            worker_name: name.into(),
            project: "/project".into(),
            session: format!("session-{id}"),
            backend: "pi".into(),
            provider: None,
            model: None,
            effort: None,
            parent_worker_id: None,
        }
    }

    #[test]
    fn post_returns_other_relevant_notices_without_internal_ids() -> Result<(), String> {
        let board = NoticeBoard::default();
        let first = caller("internal-1", "OrangeCoyote");
        let second = caller("internal-2", "SilverHeron");
        board.access(
            &first,
            Params {
                action: Action::Post,
                message: Some("editing parser".into()),
                paths: vec!["src/parser".into()],
            },
        )?;
        let response = board.access(
            &second,
            Params {
                action: Action::Post,
                message: Some("preparing parser commit".into()),
                paths: vec!["src/parser/mod.rs".into()],
            },
        )?;

        assert_eq!(response["posted"], true);
        assert_eq!(response["notices"][0]["from"], "OrangeCoyote");
        assert_eq!(response["notices"][0]["message"], "editing parser");
        assert!(!response.to_string().contains("internal-1"));
        Ok(())
    }

    #[test]
    fn reads_can_filter_unrelated_paths() -> Result<(), String> {
        let board = NoticeBoard::default();
        let first = caller("one", "OrangeCoyote");
        let second = caller("two", "SilverHeron");
        board.access(
            &first,
            Params {
                action: Action::Post,
                message: Some("editing parser".into()),
                paths: vec!["src/parser.rs".into()],
            },
        )?;
        let response = board.access(
            &second,
            Params {
                action: Action::Read,
                message: None,
                paths: vec!["src/ui".into()],
            },
        )?;
        assert_eq!(response["notices"].as_array().map(Vec::len), Some(0));
        Ok(())
    }

    #[test]
    fn children_cannot_access_the_board() {
        let board = NoticeBoard::default();
        let mut child = caller("child", "review");
        child.parent_worker_id = Some("parent".into());
        assert!(
            board
                .access(
                    &child,
                    Params {
                        action: Action::Read,
                        message: None,
                        paths: Vec::new(),
                    },
                )
                .is_err()
        );
    }
}
