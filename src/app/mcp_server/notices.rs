use std::path::{Component, Path, PathBuf};

use path_clean::PathClean as _;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::agents::CallerContext;
pub(super) use crate::app::worker_notices::NoticeBoard;
use crate::app::worker_notices::NoticeView;

const MAX_MESSAGE_BYTES: usize = 2_000;
const MAX_PATHS: usize = 64;
const MAX_PATH_BYTES: usize = 1_024;

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
    notices: Vec<NoticeResponse>,
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
        let notices = self
            .matching(&caller.project, &caller.worker_id, &paths)?
            .into_iter()
            .map(NoticeResponse::from)
            .collect();
        Ok(Response { posted, notices })
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
