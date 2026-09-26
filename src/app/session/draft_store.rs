use std::path::PathBuf;

use crate::{agents::Backend, sessions};

pub(in crate::app) fn new(
    project: PathBuf,
    harness: Option<Backend>,
    profile_id: Option<String>,
) -> Result<sessions::DraftSession, String> {
    let mut draft = sessions::DraftSession::fresh(harness, project);
    draft.profile_id = profile_id;
    let mut store = crate::app::persistence::open()?;
    draft.app_session_id = sessions::save_draft(&mut *store, &draft)?;
    Ok(draft)
}

pub(in crate::app) fn save(draft: &sessions::DraftSession) -> Result<i64, String> {
    sessions::save_draft(&mut *crate::app::persistence::open()?, draft)
}
