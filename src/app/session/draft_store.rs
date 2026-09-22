use std::path::PathBuf;

use crate::{agents::Backend, sessions};

pub(in crate::app) fn new(
    project: PathBuf,
    harness: Option<Backend>,
) -> Result<sessions::DraftSession, String> {
    let mut draft = sessions::DraftSession::fresh(harness, project);
    let mut store = crate::app::persistence::open()?;
    draft.app_session_id = sessions::save_draft(&mut *store, &draft)?;
    Ok(draft)
}

pub(in crate::app) fn save(draft: &sessions::DraftSession) -> Result<i64, String> {
    sessions::save_draft(&mut *crate::app::persistence::open()?, draft)
}

pub(in crate::app) fn remove(id: &str) -> Result<(), String> {
    sessions::remove_draft(&mut *crate::app::persistence::open()?, id)
}
