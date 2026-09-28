use std::{
    path::{Component, Path, PathBuf},
    time::Duration,
};

use path_clean::PathClean as _;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::agents::CallerContext;
pub(super) use crate::notice_board::NoticeBoard;
use crate::notice_board::NoticeView;

const MAX_MESSAGE_BYTES: usize = 2_000;
const MAX_PATHS: usize = 64;
const MAX_PATH_BYTES: usize = 1_024;

#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum Action {
    Read,
    Post,
    Wait,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(super) struct Params {
    pub(super) action: Action,
    pub(super) message: Option<String>,
    #[serde(default)]
    pub(super) paths: Vec<String>,
    #[schemars(
        description = "For wait, the cursor returned by the last read, post, or wait with the same path filter."
    )]
    pub(super) after: Option<String>,
    #[schemars(
        description = "For wait only: maximum wait in seconds, from 1 to 60; defaults to 30."
    )]
    pub(super) timeout_seconds: Option<u64>,
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct Response {
    posted: bool,
    notices: Vec<NoticeResponse>,
    cursor: String,
    timed_out: bool,
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct NoticeResponse {
    from: String,
    message: String,
    paths: Vec<String>,
    age_seconds: u64,
}

impl From<NoticeView> for NoticeResponse {
    fn from(notice: NoticeView) -> Self {
        Self {
            from: notice.from,
            message: notice.message,
            paths: notice.paths,
            age_seconds: notice.age_seconds,
        }
    }
}

impl NoticeBoard {
    pub(super) async fn access(
        &self,
        caller: &CallerContext,
        params: Params,
    ) -> Result<Response, String> {
        if caller.parent_worker_id.is_some() {
            return Err("worker notices are available only to top-level workers".into());
        }
        let action = params.action;
        let wait = matches!(action, Action::Wait);
        if wait && params.after.is_none() {
            return Err("worker notice wait requires `after` from a previous response".into());
        }
        if !wait && (params.after.is_some() || params.timeout_seconds.is_some()) {
            return Err(
                "worker notice `after` and `timeout_seconds` are valid only for wait".into(),
            );
        }
        let timeout_seconds = params.timeout_seconds.unwrap_or(30);
        if !(1..=60).contains(&timeout_seconds) {
            return Err("worker notice timeout_seconds must be between 1 and 60".into());
        }
        let paths = normalize_paths(params.paths)?;
        let message = match (action, params.message) {
            (Action::Read | Action::Wait, None) => None,
            (Action::Read | Action::Wait, Some(_)) => {
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

        let posted = message.is_some();
        if let Some(message) = message {
            self.post(
                &caller.project,
                caller.worker_id.clone(),
                caller.worker_name.clone(),
                message,
                paths.clone(),
            )?;
        }
        let batch = if let Some(after) = params.after {
            self.wait(
                &caller.project,
                &caller.worker_id,
                &paths,
                &after,
                Duration::from_secs(timeout_seconds),
            )
            .await?
        } else {
            self.matching(&caller.project, &caller.worker_id, &paths, None)?
        };
        let timed_out = wait && batch.notices.is_empty();
        let notices = batch
            .notices
            .into_iter()
            .map(NoticeResponse::from)
            .collect();
        Ok(Response {
            posted,
            notices,
            cursor: batch.cursor,
            timed_out,
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

#[cfg(test)]
#[path = "notices_tests.rs"]
mod tests;
