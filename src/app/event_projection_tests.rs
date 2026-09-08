use super::*;

#[test]
fn one_live_update_keeps_archived_rows_and_does_not_duplicate_the_session() {
    let now = std::time::SystemTime::now();
    let mut sessions = (0..3)
        .map(|index| {
            SessionSummary::from_cached(
                index.to_string(),
                PathBuf::from(format!("/sessions/{index}")),
                PathBuf::from("/project"),
                index.to_string(),
                String::new(),
                String::new(),
                None,
                now,
                0,
                crate::sessions::UsageSummary::default(),
                true,
                false,
                String::new(),
            )
        })
        .collect::<Vec<_>>();
    let mut updated = sessions[1].clone();
    updated.title = "New title".into();
    updated.modified = now + std::time::Duration::from_secs(1);
    updated.is_running = true;
    update_session_row(&mut sessions, updated.clone());
    update_session_row(&mut sessions, updated);
    assert_eq!(sessions.len(), 3);
    assert!(sessions.iter().all(|session| session.archived));
    assert_eq!(sessions[0].title, "New title");
    assert_eq!(
        sessions.iter().filter(|session| session.is_running).count(),
        1
    );
}
