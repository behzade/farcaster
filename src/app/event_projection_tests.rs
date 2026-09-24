use super::*;

#[gpui::test]
fn review_button_survives_switching_back_to_a_resident_history_snapshot(
    cx: &mut gpui::TestAppContext,
) {
    use crate::conversation::ConversationState;
    use crate::reviews::{artifact, presentation::TranscriptPresentation};
    use serde_json::json;

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::review_button_survives_switching_back_to_a_resident_history_snapshot"
        ),
        cx,
        |cx, app, runtime, project| {
            for (case, harness) in [Backend::Cursor, Backend::Antigravity]
                .into_iter()
                .enumerate()
            {
                let session = project.join(format!("review-{case}"));
                let generation = case as u64 * 3 + 1;
                let loading = RuntimeSnapshot {
                    harness: Some(harness),
                    project: project.into(),
                    selected_session: Some(session),
                    history_preview: true,
                    ..Default::default()
                };
                runtime.send_event(RuntimeEvent::Snapshot {
                    generation,
                    snapshot: Arc::new(loading.clone()),
                });
                cx.update(|_, cx| app.update(cx, |app, cx| app.drain_runtime(cx)));

                let spec = json!({"title":"Review source", "items":[
                    {"path":"src/main.rs", "note":"Inspect entry point"}
                ]});
                let arguments = match harness {
                    Backend::Cursor => json!({"providerIdentifier":"farcaster",
                        "toolName":"submit_review", "args":spec}),
                    _ => json!({"prompt":"Submitting review", "arguments":spec}),
                };
                let mut conversation = ConversationState::default();
                conversation.replace_history(&[
                    json!({"role":"assistant", "content":[{"type":"toolCall",
                        "id":"review", "name":"farcaster: submit_review", "arguments":arguments}]}),
                    json!({"role":"toolResult", "toolCallId":"review", "isError":false,
                        "content":[{"type":"text", "text":"{\"success\":true}"}]}),
                ]);
                // Runtime snapshots carry hydrated presentation rows while the
                // backend conversation retains the original lossy tool result.
                assert!(artifact::from_item(&conversation.items[0]).is_none());
                let mut presentation = TranscriptPresentation::from(&conversation);
                let mut row = conversation.items[0].as_ref().clone();
                let result = artifact::hydration_result(&row, project).expect("review hydration");
                Arc::make_mut(row.tool_details.as_mut().expect("tool details")).result =
                    Some(result);
                presentation.items.set(0, Arc::new(row));
                let loaded = Arc::new(RuntimeSnapshot {
                    conversation: Arc::new(conversation),
                    transcript: Some(Arc::new(presentation)),
                    ..loading
                });

                for (generation, snapshot, review_visible) in [
                    (generation, loaded.clone(), true),
                    (
                        generation + 1,
                        Arc::new(RuntimeSnapshot {
                            project: project.into(),
                            selected_session: Some(project.join("other")),
                            ..Default::default()
                        }),
                        false,
                    ),
                    // The supervisor returns its cached snapshot with no dirty
                    // suffix and a new selection generation.
                    (generation + 2, loaded.clone(), true),
                    (generation + 2, loaded, true),
                ] {
                    runtime.send_event(RuntimeEvent::Snapshot {
                        generation,
                        snapshot,
                    });
                    cx.update(|window, cx| {
                        app.update(cx, |app, cx| app.drain_runtime(cx));
                        window.draw(cx).clear(cx);
                    });
                    assert_eq!(
                        cx.debug_bounds("review-header-0").is_some(),
                        review_visible,
                        "review button visibility for {harness:?}, generation {generation}"
                    );
                }
            }
        },
    );
}

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
fn explicit_native_outcome_can_replace_an_earlier_explicit_outcome() {
    let activity = |outcome| {
        AgentActivity::from_native_child(
            "child".into(),
            PathBuf::from("/sessions/child"),
            "reviewer",
            false,
            Some(outcome),
        )
    };
    let mut activities = HashMap::new();
    assert!(merge_agent_activity(
        &mut activities,
        activity("complete"),
        ActivityUpdateSource::Native,
    ));

    assert!(merge_agent_activity(
        &mut activities,
        activity("incomplete"),
        ActivityUpdateSource::Native,
    ));
    assert_eq!(
        activities
            .values()
            .next()
            .expect("native activity")
            .lifecycle,
        crate::agent_activity::AgentLifecycle::Completed(
            crate::agent_activity::AgentOutcome::Incomplete
        )
    );
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
fn parked_dismissal_does_not_replace_visible_dialog() {
    let mut visible = crate::app::extensions::ExtensionUiState::default();
    visible.apply(dialog("approval"));
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
        Some("approval")
    );
    assert!(!pending_setup);
}

#[test]
fn dialogs_park_and_restore_with_history() {
    let mut visible = crate::app::extensions::ExtensionUiState::default();
    visible.apply(dialog("approval"));
    visible.apply(dialog("follow-up"));
    let mut parked = None;

    park_extension_for_history(&mut visible, &mut parked);
    assert!(visible.dialog.is_none());
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
        Some("approval")
    );
    assert_eq!(
        visible.dismiss_dialog("approval"),
        crate::app::extensions::DialogDismissal::ActiveWithNext
    );
    assert_eq!(
        visible
            .dialog
            .as_ref()
            .and_then(ExtensionUiRequest::dialog_id),
        Some("follow-up")
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
    for outcome in [
        crate::agents::PromptOutcome::Accepted,
        crate::agents::PromptOutcome::RejectedBeforeAcceptance,
        crate::agents::PromptOutcome::DeliveryUnknown,
    ] {
        for promoted_before_reply in [true, false] {
            let draft = "draft:new";
            let path = PathBuf::from("/sessions/new");
            let session = session_target(&path);
            let submission_id = "submission-new";
            let mut pending = HashMap::from([(
                submission_id.to_owned(),
                PendingSubmission {
                    id: submission_id.into(),
                    submitted_at: std::time::Instant::now(),
                    submitted_target: draft.into(),
                    mode: crate::protocol::PromptMode::Normal,
                    text: "keep this on rejection".into(),
                    images: Vec::new(),
                    pastes: Vec::new(),
                    append_on_failure: false,
                    result: None,
                },
            )]);
            if promoted_before_reply {
                pending
                    .get_mut(submission_id)
                    .expect("draft submission")
                    .submitted_target = session.clone();
            }
            record_pending_prompt_result_for_submission(
                &mut pending,
                Some(submission_id),
                draft,
                outcome,
                Some(path.clone()),
            );
            let resolved = Some((outcome, Some(path)));
            assert_eq!(pending[submission_id].result, resolved);
            // An unrelated reply must not resolve or overwrite this submission.
            record_pending_prompt_result_for_submission(
                &mut pending,
                Some("submission-other"),
                "draft:other",
                crate::agents::PromptOutcome::RejectedBeforeAcceptance,
                None,
            );
            assert_eq!(
                pending[submission_id]
                    .result
                    .as_ref()
                    .map(|result| result.0),
                resolved.map(|result| result.0)
            );
            assert_eq!(pending[submission_id].text, "keep this on rejection");
        }
    }
}

#[gpui::test]
fn promoted_draft_completion_clears_status_and_saved_attachments(cx: &mut gpui::TestAppContext) {
    use crate::app::composer::{ComposerImage, attachments};

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::promoted_draft_completion_clears_status_and_saved_attachments"
        ),
        cx,
        |cx, app, runtime, project| {
            let target = "draft:completion";
            let submission_id = "submission-completion";
            let path = project.join("completed-session");
            let session_key = session_target(&path);
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    app.begin_draft_submission(target, "hi");
                    app.composer.pending_submissions.insert(
                        submission_id.into(),
                        PendingSubmission {
                            id: submission_id.into(),
                            submitted_at: std::time::Instant::now(),
                            submitted_target: target.into(),
                            mode: crate::protocol::PromptMode::Normal,
                            text: "hi".into(),
                            images: vec![
                                ComposerImage::from_prompt(crate::protocol::PromptImage::new(
                                    "AQID".into(),
                                    "image/png".into(),
                                ))
                                .expect("test attachment"),
                            ],
                            pastes: Vec::new(),
                            append_on_failure: false,
                            result: None,
                        },
                    );
                    app.save_composer_attachments(target);
                });
            });
            let row = SessionSummary::from_cached(
                "completed".into(),
                path.clone(),
                project.into(),
                "Kickoff".into(),
                "hi".into(),
                String::new(),
                None,
                std::time::SystemTime::now(),
                0,
                crate::sessions::UsageSummary::default(),
                false,
                true,
                String::new(),
            );
            runtime.send_event(RuntimeEvent::SessionStatus {
                target: target.into(),
                session: Some(path.clone()),
                status: "Working".into(),
            });
            runtime.send_event(RuntimeEvent::SessionUpdated(row));
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.drain_runtime(cx);
                    assert_eq!(
                        app.composer.pending_submissions[submission_id].submitted_target,
                        session_key
                    );
                    assert!(!app.sessions.submitted_drafts.contains_key("completion"));
                    // Startup would restore the attachment while its UI receipt is unresolved.
                    assert_eq!(
                        attachments::restore(&app.composer.sessions).0[&session_key].len(),
                        1
                    );
                });
            });

            // The actor still addresses the original draft after the rail promotes it.
            runtime.send_event(RuntimeEvent::PromptResult {
                submission_id: Some(submission_id.into()),
                target: target.into(),
                outcome: crate::agents::PromptOutcome::Accepted,
                session: Some(path.clone()),
            });
            runtime.send_event(RuntimeEvent::SessionStatus {
                target: target.into(),
                session: Some(path.clone()),
                status: "Done".into(),
            });
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.drain_runtime(cx);
                    assert_eq!(
                        app.activity
                            .run_statuses
                            .get(&session_key)
                            .map(String::as_str),
                        Some("Done")
                    );
                });
                window.draw(cx).clear(cx);
            });
            cx.update(|_, cx| {
                let app = app.read(cx);
                assert!(app.composer.pending_submissions.is_empty());
                let (images, pastes) = attachments::restore(&app.composer.sessions);
                assert!(images.is_empty() && pastes.is_empty());
            });
        },
    );
}

#[gpui::test]
fn unknown_activity_then_real_rejection_resolves_the_original_payload_once(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::unknown_activity_then_real_rejection_resolves_the_original_payload_once"
        ),
        cx,
        |cx, app, runtime, _| {
            // A session target keeps this fixture clear of draft persistence. The
            // offline composer store is also a no-op, so this test cannot reach
            // user state or start an agent process.
            let submission_id = "submission-unknown";
            let session = PathBuf::from("/sessions/unknown");
            let target = session_target(&session);
            let image = crate::app::composer::ComposerImage::from_prompt(
            crate::protocol::PromptImage::new(
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".into(),
                "image/png".into(),
            ),
        )
        .expect("valid image");
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    app.composer.pending_submissions.insert(
                        submission_id.into(),
                        PendingSubmission {
                            id: submission_id.into(),
                            submitted_at: std::time::Instant::now(),
                            submitted_target: target.clone(),
                            mode: crate::protocol::PromptMode::Steer,
                            text: "exact unresolved text".into(),
                            images: vec![image.clone()],
                            pastes: Vec::new(),
                            append_on_failure: false,
                            result: None,
                        },
                    );
                });
            });

            for generation in 1..=2 {
                let mut conversation = crate::conversation::ConversationState::default();
                conversation.reduce(&serde_json::json!({
                "type":"prompt_delivery",
                "submissionId":submission_id,
                "status":"unknown",
                "message":{"role":"user", "content":[{"type":"text", "text":"exact unresolved text"}]},
            }));
                runtime.send_event(RuntimeEvent::Snapshot {
                    generation,
                    snapshot: Arc::new(RuntimeSnapshot {
                        conversation: Arc::new(conversation),
                        selected_session: Some(session.clone()),
                        live_session: Some(session.clone()),
                        ..RuntimeSnapshot::default()
                    }),
                });
                cx.update(|_, cx| {
                    app.update(cx, |app, cx| {
                        app.drain_runtime(cx);
                        let pending = &app.composer.pending_submissions[submission_id];
                        assert_eq!(pending.result, None);
                        assert_eq!(pending.text, "exact unresolved text");
                        assert_eq!(pending.images, std::slice::from_ref(&image));
                    });
                });
            }

            runtime.send_event(RuntimeEvent::PromptResult {
                submission_id: Some(submission_id.into()),
                target: target.clone(),
                outcome: crate::agents::PromptOutcome::RejectedBeforeAcceptance,
                session: Some(session.clone()),
            });
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.drain_runtime(cx);
                    assert_eq!(
                        app.composer.pending_submissions[submission_id].result,
                        Some((
                            crate::agents::PromptOutcome::RejectedBeforeAcceptance,
                            Some(session.clone())
                        ))
                    );
                });
                window.draw(cx).clear(cx);
            });
            cx.update(|window, cx| {
                {
                    let state = app.read(cx);
                    assert!(state.composer.pending_submissions.is_empty());
                    assert_eq!(
                        state.composer.sessions.snapshot_for(&target).text,
                        "exact unresolved text"
                    );
                    assert_eq!(state.composer.images[&target], std::slice::from_ref(&image));
                }
                window.draw(cx).clear(cx);
                let state = app.read(cx);
                assert_eq!(state.composer.images[&target], [image]);
            });
        },
    );
}

#[gpui::test]
fn child_activity_event_invalidates_and_renders_the_real_run_sidebar(
    cx: &mut gpui::TestAppContext,
) {
    use crate::sessions::UsageSummary;
    use std::time::SystemTime;

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::child_activity_event_invalidates_and_renders_the_real_run_sidebar"
        ),
        cx,
        |cx, app, runtime, project| {
            let root_path = PathBuf::from("/offline-root.jsonl");
            let child_path = PathBuf::from("/offline-child.jsonl");
            let make_session =
                |id: &str, path: PathBuf, parent_session: Option<String>, running| {
                    SessionSummary::from_cached(
                        id.into(),
                        path,
                        project.to_path_buf(),
                        id.into(),
                        String::new(),
                        String::new(),
                        parent_session,
                        SystemTime::now(),
                        0,
                        UsageSummary::default(),
                        false,
                        running,
                        String::new(),
                    )
                };
            let root = make_session("root", root_path.clone(), None, true);
            let child = make_session("child", child_path.clone(), Some("root".into()), true);
            let activity = AgentActivity::from_native_child(
                "child".into(),
                child_path.clone(),
                "worker",
                true,
                None,
            );
            let activity_key = crate::agent_activity::agent_activity_key(&child_path);
            let card_selector = "agent-card-/offline-child.jsonl";

            runtime.send_event(RuntimeEvent::Snapshot {
                generation: 1,
                snapshot: Arc::new(RuntimeSnapshot {
                    selected_session: Some(root_path.clone()),
                    live_session: Some(root_path.clone()),
                    ..RuntimeSnapshot::default()
                }),
            });
            runtime.send_event(RuntimeEvent::Sessions {
                generation: 1,
                sessions: vec![root.clone()],
                all_sessions: vec![root.clone()],
                activities: None,
            });
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.open_run_sheet(window, cx);
                    app.drain_runtime(cx);
                });
                window.draw(cx).clear(cx);
            });
            assert!(cx.debug_bounds(card_selector).is_none());

            runtime.send_event(RuntimeEvent::Sessions {
                generation: 2,
                sessions: vec![root.clone(), child.clone()],
                all_sessions: vec![root.clone(), child.clone()],
                activities: None,
            });
            runtime.send_event(RuntimeEvent::AgentActivityUpdated(activity.clone()));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
            });
            assert!(
                cx.debug_bounds(card_selector).is_some(),
                "the projected active child must render in the run sidebar"
            );
            cx.update(|_, cx| {
                let app = app.read(cx);
                assert_eq!(
                    app.activity.agents[&activity_key].lifecycle,
                    crate::agent_activity::AgentLifecycle::Working
                );
                assert_eq!(
                    app.snapshot.selected_session.as_deref(),
                    Some(root_path.as_path())
                );
            });

            let completed = AgentActivity::from_native_child(
                "child".into(),
                child_path.clone(),
                "worker",
                false,
                Some("complete"),
            );
            runtime.send_event(RuntimeEvent::AgentActivityUpdated(completed));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
            });
            assert!(
                cx.debug_bounds(card_selector).is_none(),
                "activity invalidation must remove a completed child from the collapsed section"
            );
            let inferred_incomplete =
                AgentActivity::from_native_child("child".into(), child_path, "worker", false, None);
            runtime.send_event(RuntimeEvent::AgentActivityUpdated(inferred_incomplete));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.drain_runtime(cx));
                window.draw(cx).clear(cx);
            });
            cx.update(|_, cx| {
                let activity = &app.read(cx).activity.agents[&activity_key];
                assert_eq!(
                    activity.lifecycle,
                    crate::agent_activity::AgentLifecycle::Completed(
                        crate::agent_activity::AgentOutcome::Complete
                    )
                );
                assert!(activity.explicit_outcome);
            });
        },
    );
}

#[test]
fn terminal_submission_cannot_be_overwritten_by_late_results() {
    let target = "draft:accepted";
    let submission_id = "submission-accepted";
    for (terminal, late_results) in [
        (
            crate::agents::PromptOutcome::Accepted,
            [
                crate::agents::PromptOutcome::DeliveryUnknown,
                crate::agents::PromptOutcome::RejectedBeforeAcceptance,
            ],
        ),
        (
            crate::agents::PromptOutcome::RejectedBeforeAcceptance,
            [
                crate::agents::PromptOutcome::DeliveryUnknown,
                crate::agents::PromptOutcome::Accepted,
            ],
        ),
    ] {
        let mut pending = HashMap::from([(
            submission_id.to_owned(),
            PendingSubmission {
                id: submission_id.into(),
                submitted_at: std::time::Instant::now(),
                submitted_target: target.into(),
                mode: crate::protocol::PromptMode::Steer,
                text: "terminal".into(),
                images: Vec::new(),
                pastes: Vec::new(),
                append_on_failure: false,
                result: None,
            },
        )]);
        record_pending_prompt_result_for_submission(
            &mut pending,
            Some(submission_id),
            target,
            terminal,
            None,
        );
        for late in late_results {
            record_pending_prompt_result_for_submission(
                &mut pending,
                Some(submission_id),
                target,
                late,
                None,
            );
            assert_eq!(
                pending[submission_id].result,
                Some((terminal, None)),
                "{late:?} overwrote terminal result {terminal:?}"
            );
        }
    }
}

#[test]
fn explicit_missing_submission_id_never_uses_legacy_target_fallback() {
    let target = "session:one";
    let make = |id: &str| PendingSubmission {
        id: id.into(),
        submitted_at: std::time::Instant::now(),
        submitted_target: target.into(),
        mode: crate::protocol::PromptMode::Steer,
        text: id.into(),
        images: Vec::new(),
        pastes: Vec::new(),
        append_on_failure: false,
        result: None,
    };
    let mut pending = HashMap::from([("old".into(), make("old")), ("new".into(), make("new"))]);

    record_pending_prompt_result_for_submission(
        &mut pending,
        Some("removed"),
        target,
        crate::agents::PromptOutcome::Accepted,
        None,
    );
    assert!(pending.values().all(|pending| pending.result.is_none()));
    record_pending_prompt_result_for_submission(
        &mut pending,
        None,
        target,
        crate::agents::PromptOutcome::Accepted,
        None,
    );
    assert!(pending.values().all(|pending| pending.result.is_none()));
}

#[test]
fn a_submission_id_cannot_settle_a_different_chat() {
    let mut pending = HashMap::from([(
        "id".into(),
        PendingSubmission {
            id: "id".into(),
            submitted_at: std::time::Instant::now(),
            submitted_target: "session:one".into(),
            mode: crate::protocol::PromptMode::Normal,
            text: "first chat".into(),
            images: Vec::new(),
            pastes: Vec::new(),
            append_on_failure: true,
            result: None,
        },
    )]);

    for (target, session) in [
        ("session:two", None),
        ("session:two", Some(PathBuf::from("one"))),
        ("draft:two", None),
        ("draft:two", Some(PathBuf::from("two"))),
    ] {
        record_pending_prompt_result_for_submission(
            &mut pending,
            Some("id"),
            target,
            crate::agents::PromptOutcome::Accepted,
            session,
        );

        assert!(pending["id"].result.is_none());
    }
}

#[test]
fn legacy_prompt_result_without_id_resolves_one_unambiguous_target() {
    let target = "session:one";
    let mut pending = HashMap::from([(
        "legacy".into(),
        PendingSubmission {
            id: "legacy".into(),
            submitted_at: std::time::Instant::now(),
            submitted_target: target.into(),
            mode: crate::protocol::PromptMode::Steer,
            text: "legacy prompt".into(),
            images: Vec::new(),
            pastes: Vec::new(),
            append_on_failure: false,
            result: None,
        },
    )]);
    record_pending_prompt_result_for_submission(
        &mut pending,
        None,
        target,
        crate::agents::PromptOutcome::Accepted,
        None,
    );
    assert_eq!(
        pending["legacy"].result,
        Some((crate::agents::PromptOutcome::Accepted, None))
    );
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

#[test]
fn completion_notification_is_only_redundant_for_the_visible_active_session() {
    let snapshot = RuntimeSnapshot {
        project: PathBuf::from("/project"),
        selected_session: Some(PathBuf::from("/sessions/current")),
        ..RuntimeSnapshot::default()
    };
    let current = Some((
        PathBuf::from("/sessions/current"),
        PathBuf::from("/project"),
    ));
    let background = Some((
        PathBuf::from("/sessions/background"),
        PathBuf::from("/project"),
    ));

    for (active, target, redundant) in [
        (true, current.as_ref(), true),
        (true, background.as_ref(), false),
        (false, current.as_ref(), false),
    ] {
        assert_eq!(
            completion_notification_is_redundant(active, target, &snapshot),
            redundant,
            "active={active}, target={target:?}",
        );
    }
}

#[gpui::test]
fn a_loading_chat_keeps_the_transcript_it_last_showed(cx: &mut gpui::TestAppContext) {
    use crate::conversation::ConversationState;
    use serde_json::json;

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::a_loading_chat_keeps_the_transcript_it_last_showed"
        ),
        cx,
        |cx, app, runtime, project| {
            let session = project.join("chat.jsonl");
            std::fs::write(&session, "{}").expect("write session file");
            let mut conversation = ConversationState::default();
            conversation.replace_history(&[
                json!({"role":"user", "content":"hello"}),
                json!({"role":"assistant", "content":[{"type":"text", "text":"hi"}]}),
            ]);
            runtime.send_event(RuntimeEvent::Snapshot {
                generation: 1,
                snapshot: Arc::new(RuntimeSnapshot {
                    project: project.to_path_buf(),
                    selected_session: Some(session.clone()),
                    conversation: Arc::new(conversation),
                    status: "Ready".into(),
                    ..Default::default()
                }),
            });
            cx.update(|_, cx| app.update(cx, |app, cx| app.drain_runtime(cx)));
            cx.update(|_, cx| {
                assert_eq!(app.read(cx).snapshot.conversation.items.len(), 2);
            });

            // Selecting the same chat again clears the runtime's snapshot until
            // the history load finishes.
            runtime.send_event(RuntimeEvent::Snapshot {
                generation: 2,
                snapshot: Arc::new(RuntimeSnapshot {
                    project: project.to_path_buf(),
                    selected_session: Some(session),
                    status: "Loading history".into(),
                    ..Default::default()
                }),
            });
            cx.update(|_, cx| app.update(cx, |app, cx| app.drain_runtime(cx)));
            cx.update(|_, cx| {
                assert_eq!(
                    app.read(cx).snapshot.conversation.items.len(),
                    2,
                    "a loading snapshot must not clear the transcript on screen",
                );
            });
        },
    );
}
