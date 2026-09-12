use super::*;

#[test]
fn metadata_refresh_does_not_erase_an_explicit_native_outcome() {
    let child = |running: bool, outcome: Option<&str>| {
        serde_json::json!({
            "harness": "codex-cli", "id": "child", "path": "/sessions/child",
            "project": "/project", "title": "reviewer", "first_user_message": null,
            "parent_session": "parent", "message_count": null, "model": null,
            "thinking_level": null, "service_tier": null, "usage": null,
            "is_running": running, "outcome": outcome,
        })
    };
    let completed_event = child(false, Some("complete"));
    let completed_metadata = serde_json::from_value(completed_event.clone()).expect("metadata");
    let completed =
        crate::app::runtime::catalog::native_child_activity(&completed_event, &completed_metadata);
    let idle_event = child(false, None);
    let idle_metadata = serde_json::from_value(idle_event.clone()).expect("metadata");
    let metadata_only =
        crate::app::runtime::catalog::native_child_activity(&idle_event, &idle_metadata);

    let mut activities = HashMap::new();
    assert!(merge_agent_activity(
        &mut activities,
        completed.clone(),
        ActivityUpdateSource::Native,
    ));
    assert!(!merge_agent_activity(
        &mut activities,
        metadata_only.clone(),
        ActivityUpdateSource::Native,
    ));
    assert_eq!(
        activities
            .values()
            .next()
            .expect("native activity")
            .lifecycle,
        completed.lifecycle
    );
    assert_eq!(
        crate::app::views::run_panel::agents::agent_section(
            activities.values().next().expect("activity").lifecycle,
            true,
            false,
        ),
        crate::app::views::run_panel::agents::AgentSection::Completed
    );

    let mut running_metadata = metadata_only;
    running_metadata.lifecycle = crate::agent_activity::AgentLifecycle::Working;
    assert!(!merge_agent_activity(
        &mut activities,
        running_metadata,
        ActivityUpdateSource::Metadata,
    ));
}

#[test]
fn scoped_paths_keep_same_native_id_in_separate_rows_and_focus_targets() {
    let activity = |path: &str| {
        AgentActivity::from_native_child(
            "same-native-id".into(),
            PathBuf::from(path),
            "worker",
            true,
            None,
        )
    };
    let mut activities = HashMap::new();
    assert!(merge_agent_activity(
        &mut activities,
        activity("/one/child"),
        ActivityUpdateSource::Native,
    ));
    assert!(merge_agent_activity(
        &mut activities,
        activity("/two/child"),
        ActivityUpdateSource::Native,
    ));

    assert_eq!(activities.len(), 2);
    assert!(
        activities.contains_key(&crate::agent_activity::agent_activity_key(Path::new(
            "/one/child"
        )))
    );
    assert!(
        activities.contains_key(&crate::agent_activity::agent_activity_key(Path::new(
            "/two/child"
        )))
    );
}

#[test]
fn catalog_lifecycle_updates_preserve_richer_activity_details() {
    let mut rich = AgentActivity::from_native_child(
        "child".into(),
        PathBuf::from("/sessions/child"),
        "worker",
        true,
        None,
    );
    rich.limited = false;
    rich.activity = "Inspect the real transport".into();
    rich.current_tool = Some(crate::agent_activity::AgentToolActivity {
        name: "read".into(),
        target: "src/main.rs".into(),
        failed: false,
    });
    let mut pool = rich.clone();
    pool.limited = true;
    pool.lifecycle = crate::agent_activity::AgentLifecycle::NeedsInput;
    pool.activity.clear();
    pool.current_tool = None;
    let mut activities = HashMap::new();
    merge_agent_activity(&mut activities, rich, ActivityUpdateSource::Native);

    assert!(merge_agent_activity(
        &mut activities,
        pool,
        ActivityUpdateSource::Catalog,
    ));
    let merged = activities.values().next().expect("merged activity");
    assert_eq!(
        merged.lifecycle,
        crate::agent_activity::AgentLifecycle::NeedsInput
    );
    assert_eq!(merged.activity, "Inspect the real transport");
    assert_eq!(
        merged.current_tool.as_ref().map(|tool| tool.name.as_str()),
        Some("read")
    );
    assert!(!merged.limited);
}

fn dialog(id: &str) -> ExtensionUiRequest {
    ExtensionUiRequest::Input {
        id: id.into(),
        title: id.into(),
        placeholder: None,
        timeout: None,
    }
}

#[test]
fn dialog_dismissal_requires_the_current_generation_and_schedules_focus_lifecycle() {
    let mut extension = crate::app::extensions::ExtensionUiState::default();
    extension.apply(dialog("active"));
    extension.apply(dialog("expired-queued"));
    extension.apply(dialog("next"));
    let mut restored = Some("active".into());
    let mut dismissed = None;
    let mut pending_setup = false;

    assert!(!project_dialog_dismissal(
        4,
        5,
        "active",
        &mut extension,
        None,
        &mut restored,
        &mut dismissed,
        &mut pending_setup,
    ));
    assert_eq!(
        extension
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id),
        Some("active")
    );
    assert!(!pending_setup);

    assert!(!project_dialog_dismissal(
        5,
        5,
        "expired-queued",
        &mut extension,
        None,
        &mut restored,
        &mut dismissed,
        &mut pending_setup,
    ));
    assert_eq!(
        extension
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id),
        Some("active")
    );
    assert!(
        !pending_setup,
        "queued removal must not disturb current focus"
    );

    assert!(project_dialog_dismissal(
        5,
        5,
        "active",
        &mut extension,
        None,
        &mut restored,
        &mut dismissed,
        &mut pending_setup,
    ));
    assert_eq!(
        extension
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id),
        Some("next")
    );
    assert!(
        pending_setup,
        "root lifecycle must focus the promoted dialog"
    );
    assert_eq!(restored, None);
    assert_eq!(dismissed.as_deref(), Some("active"));

    pending_setup = false;
    assert!(project_dialog_dismissal(
        5,
        5,
        "next",
        &mut extension,
        None,
        &mut restored,
        &mut dismissed,
        &mut pending_setup,
    ));
    assert!(extension.dialog.is_none());
    assert!(
        pending_setup,
        "root lifecycle must restore focus after the final dialog"
    );
}

#[test]
fn parked_dismissal_does_not_replace_visible_recovery() {
    let mut visible = crate::app::extensions::ExtensionUiState::default();
    visible.apply(dialog("farcaster-recovery-9"));
    let mut parked = crate::app::extensions::ExtensionUiState::default();
    parked.apply(dialog("expired-child"));
    let mut restored = None;
    let mut dismissed = None;
    let mut pending_setup = false;

    assert!(!project_dialog_dismissal(
        2,
        2,
        "expired-child",
        &mut visible,
        Some(&mut parked),
        &mut restored,
        &mut dismissed,
        &mut pending_setup,
    ));
    assert!(parked.dialog.is_none());
    assert_eq!(
        visible
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id),
        Some("farcaster-recovery-9")
    );
    assert!(!pending_setup);
}

#[test]
fn recovery_dialog_stays_visible_across_history_parking_and_restore() {
    let mut visible = crate::app::extensions::ExtensionUiState::default();
    visible.apply(dialog("approval"));
    visible.apply(dialog("farcaster-recovery-4"));
    let mut parked = None;

    park_extension_for_history(&mut visible, &mut parked);
    assert_eq!(
        visible
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id),
        Some("farcaster-recovery-4")
    );
    assert_eq!(
        parked
            .as_ref()
            .and_then(|state| state.dialog.as_ref())
            .and_then(ExtensionUiRequest::dialog_id),
        Some("approval")
    );

    restore_extension_after_history(&mut visible, &mut parked);
    assert!(parked.is_none());
    assert_eq!(
        visible
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id),
        Some("farcaster-recovery-4")
    );
    assert_eq!(
        visible.dismiss_dialog("farcaster-recovery-4"),
        crate::app::extensions::DialogDismissal::ActiveWithNext
    );
    assert_eq!(
        visible
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id),
        Some("approval")
    );
}

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
