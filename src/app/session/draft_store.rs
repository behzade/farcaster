use std::path::PathBuf;

use crate::{agents::Backend, sessions};

pub(in crate::app) fn new(
    project: PathBuf,
    harness: Option<Backend>,
    profile_id: Option<String>,
) -> sessions::DraftSession {
    let mut draft = sessions::DraftSession::fresh(harness, project);
    draft.profile_id = profile_id;
    draft
}

pub(in crate::app) fn save(draft: &sessions::DraftSession) -> Result<i64, String> {
    sessions::save_draft(&mut *crate::app::persistence::open()?, draft)
}
