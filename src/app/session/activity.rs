use std::{collections::HashMap, path::Path};

use crate::{app::composer::sessions::session_target, runtime::RuntimeSnapshot};

pub(in crate::app) fn status_has_active_work(status: &str) -> bool {
    matches!(
        status,
        "Working" | "Compacting" | "Retrying" | "Needs input"
    )
}

pub(in crate::app) fn snapshot_has_active_work(snapshot: &RuntimeSnapshot) -> bool {
    !snapshot.history_preview
        && (snapshot.conversation.running
            || snapshot.conversation.compacting
            || snapshot.conversation.retrying
            || snapshot.pending_question.is_some())
}

pub(in crate::app) fn session_has_live_work(
    path: &Path,
    statuses: &HashMap<String, String>,
    snapshot: &RuntimeSnapshot,
) -> bool {
    statuses
        .get(&session_target(path))
        .is_some_and(|status| status_has_active_work(status))
        || (snapshot
            .live_session
            .as_deref()
            .or(snapshot.selected_session.as_deref())
            == Some(path)
            && snapshot_has_active_work(snapshot))
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
