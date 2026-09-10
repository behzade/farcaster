use super::*;

#[test]
fn stopping_selected_session_clears_archive_active_work_flags() {
    let path = PathBuf::from("/sessions/stopped");
    let mut snapshot = RuntimeSnapshot {
        live_session: Some(path.clone()),
        selected_session: Some(path.clone()),
        connected: true,
        ..RuntimeSnapshot::default()
    };
    let conversation = Arc::make_mut(&mut snapshot.conversation);
    conversation.running = true;
    conversation.compacting = true;
    conversation.retrying = true;
    clear_stopped_snapshot(&mut snapshot, Path::new("/sessions/other"));
    assert!(crate::app::session::activity::snapshot_has_active_work(
        &snapshot
    ));
    clear_stopped_snapshot(&mut snapshot, &path);
    assert!(!crate::app::session::activity::snapshot_has_active_work(
        &snapshot
    ));
    assert!(!snapshot.connected);
    assert_eq!(snapshot.status, "Stopped");
}

#[test]
fn prompt_result_follows_submission_through_draft_promotion() {
    for accepted in [true, false] {
        for promoted_before_reply in [true, false] {
            let draft = "draft:new";
            let path = PathBuf::from("/sessions/new");
            let session = session_target(&path);
            let mut pending = HashMap::from([(
                draft.to_owned(),
                PendingSubmission {
                    submitted_target: draft.into(),
                    text: "keep this on rejection".into(),
                    images: Vec::new(),
                    pastes: Vec::new(),
                    append_on_failure: false,
                    result: None,
                },
            )]);
            if promoted_before_reply {
                let submission = pending.remove(draft).expect("draft submission");
                pending.insert(session.clone(), submission);
            }
            record_pending_prompt_result(&mut pending, draft, accepted, Some(path.clone()));
            let key = if promoted_before_reply {
                &session
            } else {
                draft
            };
            assert_eq!(pending[key].result, Some((accepted, Some(path))));
            // An unrelated reply must not resolve or overwrite this submission.
            record_pending_prompt_result(&mut pending, "draft:other", !accepted, None);
            assert_eq!(
                pending[key].result.as_ref().map(|result| result.0),
                Some(accepted)
            );
            assert_eq!(pending[key].text, "keep this on rejection");
        }
    }
}

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
