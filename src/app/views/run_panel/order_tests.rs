use super::ordered_worker_rows;
use crate::sessions::SessionSummary;
use std::{
    path::Path,
    time::{Duration, SystemTime},
};

fn session(id: &str, timestamp: &str, parent: Option<&str>) -> SessionSummary {
    SessionSummary::from_cached(
        id.into(),
        format!("/project/{id}").into(),
        "/project".into(),
        "worker".into(),
        String::new(),
        timestamp.into(),
        parent.map(str::to_owned),
        SystemTime::UNIX_EPOCH,
        0,
        Default::default(),
        false,
        false,
        String::new(),
    )
}

fn worker_ids(sessions: &[SessionSummary]) -> Vec<String> {
    ordered_worker_rows(
        &crate::sessions::SessionCatalog::from(sessions.to_vec()),
        &Default::default(),
        Some(Path::new("/project/root")),
    )
    .into_iter()
    .map(|(_, _, session, _)| session.id.clone())
    .collect()
}

#[test]
fn worker_order_falls_back_to_descending_session_id() {
    for timestamp in ["", "invalid", "2026-09-23T00:00:00Z"] {
        let sessions = std::iter::once(session("root", "", None))
            .chain(["b", "a", "d", "c"].map(|id| session(id, timestamp, Some("root"))))
            .collect::<Vec<_>>();
        assert_eq!(worker_ids(&sessions), ["d", "c", "b", "a"]);
    }
}

#[test]
fn worker_order_uses_creation_time_across_formats_and_stored_fallbacks() {
    let mut sessions = vec![
        session("root", "", None),
        session("z", "2026-09-23T01:00:01+01:00", Some("root")),
        session("b", "", Some("root")),
        session("x", "invalid", Some("root")),
        session("a", "2026-09-23T00:00:04Z", Some("root")),
    ];
    let base =
        crate::agent_activity::parse_iso_timestamp("2026-09-23T00:00:00Z").expect("creation time");
    sessions[2].created_at = Some(base + Duration::from_secs(2));
    sessions[3].created_at = Some(base + Duration::from_secs(3));
    // Recent activity on an older worker must not move it above newer workers.
    sessions[1].modified = SystemTime::now();
    assert_eq!(worker_ids(&sessions), ["a", "x", "b", "z"]);
}
