use std::{path::PathBuf, sync::Arc};

use super::*;

#[test]
fn active_statuses_guard_archive_and_quit_without_catalog_activity() {
    let path = PathBuf::from("/sessions/codex");
    for status in ["Working", "Compacting", "Retrying", "Needs input"] {
        let statuses = HashMap::from([(session_target(&path), status.to_owned())]);
        assert!(session_has_live_work(
            &path,
            &statuses,
            &RuntimeSnapshot::default()
        ));
        assert!(!session_has_live_work(
            Path::new("/sessions/other"),
            &statuses,
            &RuntimeSnapshot::default()
        ));
    }
    for status in ["Done", "Failed", "Stopped", "Idle", "Ready", "Draft"] {
        assert!(!status_has_active_work(status));
    }
}

#[test]
fn live_snapshot_guards_before_status_arrives_and_ignores_history_preview() {
    let path = PathBuf::from("/sessions/codex");
    for state in ["running", "compacting", "retrying"] {
        let mut snapshot = RuntimeSnapshot {
            live_session: Some(path.clone()),
            selected_session: Some(PathBuf::from("/sessions/preview")),
            ..Default::default()
        };
        let conversation = Arc::make_mut(&mut snapshot.conversation);
        conversation.running = state == "running";
        conversation.compacting = state == "compacting";
        conversation.retrying = state == "retrying";
        assert!(snapshot_has_active_work(&snapshot));
        assert!(session_has_live_work(&path, &HashMap::new(), &snapshot));
        assert!(!session_has_live_work(
            Path::new("/sessions/preview"),
            &HashMap::new(),
            &snapshot
        ));
        snapshot.history_preview = true;
        assert!(!snapshot_has_active_work(&snapshot));
    }
}
