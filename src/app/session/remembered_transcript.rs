use super::remembered::{Remembered, Stamp};
use super::*;
use std::sync::{Mutex, MutexGuard};

const LIMIT: usize = 8;

static CACHE: Mutex<Remembered<Arc<RuntimeSnapshot>>> = Mutex::new(Remembered::new(LIMIT));

fn cache() -> MutexGuard<'static, Remembered<Arc<RuntimeSnapshot>>> {
    CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(in crate::app) fn remember(snapshot: &Arc<RuntimeSnapshot>) {
    let Some(path) = snapshot.selected_session.clone() else {
        return;
    };
    if snapshot.status == "Loading history" {
        return;
    }
    if snapshot.status != "Ready" || snapshot.conversation.items.is_empty() {
        cache().forget(&path);
        return;
    }
    let stamp = Stamp::of(&path);
    cache().remember(path, stamp, snapshot);
}

pub(in crate::app) fn stand_in(snapshot: Arc<RuntimeSnapshot>) -> Arc<RuntimeSnapshot> {
    if snapshot.status != "Loading history" || !snapshot.conversation.items.is_empty() {
        return snapshot;
    }
    let Some(path) = snapshot.selected_session.clone() else {
        return snapshot;
    };
    let Some(remembered) = cache().recall(&path, Stamp::of(&path).as_ref()) else {
        return snapshot;
    };
    if remembered.project != snapshot.project
        || remembered.harness != snapshot.harness
        || remembered.profile_id != snapshot.profile_id
    {
        return snapshot;
    }
    Arc::new(RuntimeSnapshot {
        conversation: remembered.conversation.clone(),
        transcript: remembered.transcript.clone(),
        stats: remembered.stats.clone(),
        transcript_changed_from: Some(0),
        ..(*snapshot).clone()
    })
}

#[cfg(test)]
#[path = "remembered_transcript_tests.rs"]
mod tests;
