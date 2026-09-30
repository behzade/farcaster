use super::*;
use crate::agents::Backend;

#[gpui::test]
fn active_profile_uses_saved_native_session_identity_and_runtime_fallback(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::active_profile_uses_saved_native_session_identity_and_runtime_fallback"
        ),
        cx,
        |cx, app, _, project| {
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    let profile = "00000000-0000-4000-8000-000000000001";
                    let mut draft = DraftSession::fresh(Some(Backend::Pi), project.into());
                    draft.profile_id = Some(profile.into());
                    draft.submitted = true;
                    draft.session_path = Some(normalize_session_path(
                        &project.join("native-session.jsonl"),
                    ));
                    let mut store = crate::app::persistence::open().expect("store");
                    draft.app_session_id =
                        store.allocate_app_session_id(&draft).expect("bind draft");
                    app.sessions.all = store.cached_sessions("").expect("sessions").into();
                    app.sessions.selected_draft = None;
                    let snapshot = std::sync::Arc::make_mut(&mut app.snapshot);
                    snapshot.selected_session = draft.session_path.clone();
                    snapshot.profile_id = Some("runtime-profile".into());
                    assert_eq!(app.active_profile_id().as_deref(), Some(profile));
                    app.sessions
                        .all
                        .iter_mut()
                        .find(|session| session.app_session_id == draft.app_session_id)
                        .expect("selected session")
                        .profile_id = None;
                    assert_eq!(
                        app.active_profile_id(),
                        None,
                        "a known built-in session owns its profile choice"
                    );
                    app.sessions.all.clear();
                    assert_eq!(app.active_profile_id().as_deref(), Some("runtime-profile"));

                    app.sessions.selected_draft = Some(draft.id.clone());
                    app.sessions.drafts.insert(0, draft);
                    assert_eq!(app.active_profile_id().as_deref(), Some(profile));
                    app.sessions.drafts[0].profile_id = None;
                    assert_eq!(
                        app.active_profile_id(),
                        None,
                        "a built-in draft owns its profile choice"
                    );
                });
            });
        },
    );
}

#[gpui::test]
fn draft_archive_sends_durable_intent_without_a_ui_locator(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::draft_archive_sends_durable_intent_without_a_ui_locator"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    futures::executor::block_on(app.sessions.writer.flush())
                        .expect("initial saves");
                    for bound in [false, true] {
                        for archived in [false, true] {
                            let mut draft = DraftSession::fresh(Some(Backend::Pi), project.into());
                            draft.submitted = true;
                            draft.archived = !archived;
                            draft.app_session_id =
                                super::super::draft_store::save(&draft).expect("save draft");
                            let mut store = crate::app::persistence::open().expect("store");
                            if bound {
                                let mut canonical = draft.clone();
                                canonical.session_path =
                                    Some(project.join(format!("{}.jsonl", draft.id)));
                                store
                                    .allocate_app_session_id(&canonical)
                                    .expect("runtime binding");
                            }
                            app.sessions.drafts.insert(0, draft.clone());
                            while runtime.try_recv_command().is_some() {}
                            app.request_draft_archive(draft.id.clone(), archived, window, cx);
                            assert_eq!(app.sessions.drafts[0].archived, archived);
                            let command = runtime.try_recv_command().expect("explicit intent");
                            let RuntimeCommand::SetAppSessionArchived {
                                app_session_id,
                                archived: requested,
                            } = command
                            else {
                                panic!("archive must use durable identity");
                            };
                            assert_eq!(app_session_id.get(), draft.app_session_id);
                            assert_eq!(requested, archived);
                            let saved = store
                                .load_drafts()
                                .expect("drafts")
                                .into_iter()
                                .find(|saved| saved.id == draft.id)
                                .expect("saved draft");
                            assert_eq!(saved.archived, !archived);
                            store
                                .set_app_session_archived(app_session_id, requested)
                                .expect("apply intent");
                            let saved = store
                                .load_drafts()
                                .expect("drafts")
                                .into_iter()
                                .find(|saved| saved.id == draft.id)
                                .expect("saved draft");
                            assert_eq!(saved.archived, archived);
                        }
                    }
                });
            });
        },
    );
}

#[test]
fn startup_idle_preserves_only_the_unresolved_draft_submission() {
    use crate::{agents::PromptOutcome, protocol::PromptMode};

    let target = draft_target("starting");
    let path = PathBuf::from("/sessions/starting");
    let session_key = session_target(&path);
    let mut pending = HashMap::from([(
        "submission".into(),
        PendingSubmission {
            id: "submission".into(),
            submitted_at: std::time::Instant::now(),
            submitted_target: target.clone(),
            mode: PromptMode::Normal,
            text: "hello".into(),
            images: Vec::new(),
            pastes: Vec::new(),
            append_on_failure: false,
            result: None,
        },
    )]);
    let mut statuses = HashMap::from([(target.clone(), "Working".into())]);
    let preserves = |target: &str, status: &str, pending: &_, statuses: &_| {
        preserve_submission_working_status(target, Some(&path), status, pending, statuses)
    };
    assert!(preserves(&target, "Done", &pending, &statuses));
    assert!(!preserves("draft:other", "Done", &pending, &statuses));
    assert!(!preserves(&session_key, "Done", &pending, &statuses));
    for status in ["Failed", "Stopped", "Delivery unknown"] {
        assert!(!preserves(&target, status, &pending, &statuses));
    }

    transfer_draft_status(&mut statuses, &mut HashMap::new(), "starting", &path);
    pending
        .get_mut("submission")
        .expect("pending submission")
        .submitted_target = session_key.clone();
    assert!(preserves(&target, "Done", &pending, &statuses));
    for terminal in ["Done", "Failed", "Stopped", "Delivery unknown"] {
        statuses.insert(session_key.clone(), terminal.into());
        assert!(!preserves(&target, "Done", &pending, &statuses));
    }
    statuses.insert(session_key, "Working".into());
    for outcome in [
        PromptOutcome::Accepted,
        PromptOutcome::RejectedBeforeAcceptance,
        PromptOutcome::DeliveryUnknown,
    ] {
        pending
            .get_mut("submission")
            .expect("pending submission")
            .result = Some((outcome, Some(path.clone())));
        assert!(!preserves(&target, "Done", &pending, &statuses));
    }
    pending.clear();
    assert!(!preserves(&target, "Done", &pending, &statuses));
}

#[test]
fn empty_startup_draft_stays_deleted_after_late_composer_save()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::app::infrastructure::persistence::{ComposerRecord, StateStore};

    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let project = temp.path().canonicalize()?;
    let mut store = StateStore::open_at(&database)?;
    let draft = DraftSession::with_id(Some(Backend::Pi), "startup".into(), project.clone());
    let id = store.allocate_app_session_id(&draft)?;
    let mut drafts = store.load_drafts()?;
    sync_materialized_draft(&mut drafts, "startup", id, &project, Some(Backend::Pi));
    store.remove_draft("startup")?;
    store.save_composer_session(&ComposerRecord {
        target: draft_target("startup"),
        ..Default::default()
    })?;
    drop(store);

    let reopened = StateStore::open_at(&database)?;
    assert!(reopened.load_drafts()?.is_empty());
    assert!(reopened.load_composer_sessions()?.is_empty());
    Ok(())
}

#[test]
fn project_choices_include_registered_and_current_worktrees() {
    let temp = tempfile::tempdir().expect("temporary project root");
    let project = temp.path().join("project");
    let other = temp.path().join("other");
    let worktree = temp.path().join("worktree");
    let worktree_git_dir = project.join(".git/worktrees/feature");
    std::fs::create_dir_all(&worktree_git_dir).expect("worktree metadata");
    std::fs::create_dir_all(&worktree).expect("worktree directory");
    std::fs::write(worktree_git_dir.join("commondir"), "../..\n")
        .expect("worktree common directory pointer");
    std::fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", worktree_git_dir.display()),
    )
    .expect("worktree git pointer");
    let registered = vec![project.clone(), worktree.clone(), other.clone()];

    assert_eq!(
        available_projects(&registered, &other),
        vec![other.clone(), worktree.clone(), project.clone()]
    );
    assert_eq!(
        available_projects(&registered, &worktree),
        vec![worktree, project, other]
    );
}

#[test]
fn drafts_materialize_once_and_survive_leaving_them() {
    let project = PathBuf::from("/project");
    let mut drafts = Vec::new();

    assert!(sync_materialized_draft(
        &mut drafts,
        "first",
        42,
        &project,
        Some(Backend::Codex),
    ));
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].id, "first");
    assert_eq!(drafts[0].app_session_id, 42);
    assert_eq!(drafts[0].harness, Some(Backend::Codex));
    assert!(!sync_materialized_draft(
        &mut drafts,
        "first",
        42,
        &project,
        Some(Backend::Codex),
    ));
    assert!(sync_materialized_draft(
        &mut drafts,
        "second",
        43,
        &project,
        Some(Backend::Codex),
    ));
    assert_eq!(drafts.len(), 2);
    assert!(drafts.iter().any(|draft| draft.id == "first"));
}

#[test]
fn provisional_title_uses_first_nonblank_bounded_prompt_line() {
    assert_eq!(
        provisional_session_title("\n  Fix the composer submission flow.\nMore detail"),
        Some("Fix the composer submission flow".into())
    );
    assert_eq!(provisional_session_title("   \n"), None);
    assert_eq!(
        provisional_session_title(
            "one two three four five six seven eight nine ten eleven twelve thirteen"
        ),
        Some("one two three four five six seven eight nine ten eleven twelve".into())
    );
}

#[test]
fn submitted_pathless_drafts_keep_their_pending_identity() {
    let draft = DraftSession {
        id: "pending".into(),
        app_session_id: 1,
        harness: Some(Backend::Pi),
        profile_id: None,
        project: PathBuf::from("/project"),
        created_ms: 1,
        submitted: true,
        session_path: None,
        title: Some("Pending session".into()),
        archived: false,
    };

    assert_eq!(
        submitted_draft_associations(&[draft]),
        HashMap::from([("pending".into(), None)])
    );
}

#[test]
fn submitted_a_and_selected_empty_b_keep_distinct_identity() {
    let path = PathBuf::from("/sessions/a.jsonl");
    let mut submitted = HashMap::new();

    assert_eq!(
        establish_submission(&mut submitted, "draft:a", true, Some(path.clone()),),
        Some("a".into())
    );
    let selected_draft = "b";

    assert_eq!(selected_draft, "b");
    assert_eq!(submitted.get("a"), Some(&Some(path)));
    assert_eq!(
        resolved_draft_status("a", &submitted, &HashMap::new()),
        "Working"
    );
    assert_eq!(
        resolved_draft_status("b", &submitted, &HashMap::new()),
        "Draft"
    );
}

#[test]
fn later_draft_status_fills_only_an_established_submission() {
    let path = PathBuf::from("/sessions/a.jsonl");
    let mut submitted = HashMap::new();
    establish_submission(&mut submitted, "draft:a", true, None);

    assert_eq!(
        fill_session_association(&mut submitted, "draft:a", Some(&path)),
        Some(path.clone())
    );
    assert_eq!(
        fill_session_association(&mut submitted, "draft:b", Some(&path)),
        None
    );
    assert!(!submitted.contains_key("b"));
}

#[test]
fn submitted_draft_status_prefers_draft_then_associated_session_then_fallback() {
    let path = PathBuf::from("/sessions/a.jsonl");
    let submitted = HashMap::from([("a".into(), Some(path.clone()))]);
    let session_key = session_target(&path);
    let mut statuses = HashMap::from([(session_key, "Needs input".into())]);

    assert_eq!(
        resolved_draft_status("a", &submitted, &statuses),
        "Needs input"
    );
    statuses.insert(draft_target("a"), "Failed".into());
    assert_eq!(resolved_draft_status("a", &submitted, &statuses), "Failed");
    statuses.remove(&draft_target("a"));
    statuses.insert(session_target(&path), "Done".into());
    assert_eq!(resolved_draft_status("a", &submitted, &statuses), "Done");
    statuses.insert(session_target(&path), "Working".into());
    assert_eq!(resolved_draft_status("a", &submitted, &statuses), "Working");
}

#[test]
fn accepted_draft_with_exact_path_reconciles_after_store_reopen()
-> Result<(), Box<dyn std::error::Error>> {
    use std::{fs, time::SystemTime};

    use tempfile::tempdir;

    use crate::{
        app::infrastructure::persistence::StateStore,
        projects::Registry,
        sessions::{SessionSummary, UsageSummary},
    };

    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let session = temp.path().join("a.jsonl");
    fs::write(&session, "{}")?;
    let project = project.canonicalize()?;
    let session = session.canonicalize()?;
    let mut drafts = vec![DraftSession {
        id: "a".into(),
        app_session_id: 1,
        harness: Some(Backend::Pi),
        profile_id: None,
        project: project.clone(),
        created_ms: 1,
        submitted: false,
        session_path: None,
        title: None,
        archived: false,
    }];
    let mut submitted = HashMap::new();

    establish_submission(&mut submitted, "draft:a", true, Some(session.clone()));
    assert!(update_persisted_submission(
        &mut drafts,
        "a",
        Some(&session)
    ));
    let database = temp.path().join("gui-state.sqlite3");
    {
        let mut store = StateStore::open_at(&database)?;
        store.save_registry(&Registry {
            projects: vec![project.clone()],
            excluded_projects: Vec::new(),
            drafts,
        })?;
        store.replace_sessions(&[SessionSummary::from_cached(
            "session-a".into(),
            session.clone(),
            project,
            "Session A".into(),
            "hello".into(),
            "2026-08-15T00:00:00Z".into(),
            None,
            SystemTime::now(),
            1,
            UsageSummary::default(),
            false,
            false,
            "session a hello".into(),
        )])?;
    }

    let store = StateStore::open_at(&database)?;
    let restarted = store.load_registry()?;
    let restarted_submitted = submitted_draft_associations(&restarted.drafts);
    let catalog = store.cached_sessions("")?;

    assert_eq!(
        reconciliation_candidates(
            &restarted_submitted,
            catalog.iter().map(|summary| summary.path.as_path()),
        ),
        vec![("a".into(), session)]
    );
    Ok(())
}

#[test]
fn accepted_draft_without_a_path_is_never_durable() {
    let mut drafts = vec![DraftSession {
        id: "a".into(),
        app_session_id: 1,
        harness: Some(Backend::Pi),
        profile_id: None,
        project: PathBuf::from("/project"),
        created_ms: 1,
        submitted: false,
        session_path: None,
        title: None,
        archived: false,
    }];
    let mut submitted = HashMap::new();

    establish_submission(&mut submitted, "draft:a", true, None);

    assert_eq!(submitted.get("a"), Some(&None));
    assert!(!update_persisted_submission(&mut drafts, "a", None));
    assert!(!drafts[0].submitted);
    assert_eq!(drafts[0].session_path, None);
    assert!(submitted_draft_associations(&drafts).is_empty());
}

#[test]
fn background_submitted_draft_reconciles_while_b_stays_selected() {
    let path = PathBuf::from("/sessions/a.jsonl");
    let submitted = HashMap::from([("a".into(), Some(path.clone()))]);
    let mut selected_draft = Some("b".to_owned());

    assert_eq!(
        reconciliation_candidates(&submitted, [path.as_path()].into_iter()),
        vec![("a".into(), path)]
    );
    clear_promoted_selection(&mut selected_draft, "a");
    assert_eq!(selected_draft.as_deref(), Some("b"));
}

#[test]
fn promotion_transfers_working_status_to_one_canonical_session_key() {
    let path = PathBuf::from("/sessions/a.jsonl");
    let draft_key = draft_target("a");
    let session_key = session_target(&path);
    let mut statuses = HashMap::from([
        (draft_key.clone(), "Working".into()),
        (session_key.clone(), "Working".into()),
    ]);
    let mut completions = HashMap::new();

    transfer_draft_status(&mut statuses, &mut completions, "a", &path);

    assert_eq!(
        statuses.get(&session_key).map(String::as_str),
        Some("Working")
    );
    assert!(!statuses.contains_key(&draft_key));
    assert_eq!(statuses.len(), 1);
}

#[test]
fn reconciliation_requires_an_exact_discovered_path() {
    let path = PathBuf::from("/sessions/a.jsonl");
    let submitted = HashMap::from([
        ("a".into(), Some(path)),
        ("b".into(), Some(PathBuf::from("/sessions/b.jsonl"))),
    ]);

    assert!(
        reconciliation_candidates(
            &submitted,
            [std::path::Path::new("/sessions/other.jsonl")].into_iter(),
        )
        .is_empty()
    );
}

#[test]
fn materialized_codex_draft_can_enqueue_without_a_duplicate_client_key()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::app::infrastructure::persistence::StateStore;
    let temp = tempfile::tempdir()?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let draft = DraftSession::with_id(
        Some(Backend::Codex),
        "codex-draft".into(),
        temp.path().to_owned(),
    );
    let id = store.allocate_app_session_id(&draft)?;
    let mut drafts = Vec::new();
    sync_materialized_draft(
        &mut drafts,
        &draft.id,
        id,
        temp.path(),
        Some(Backend::Codex),
    );
    store.save_registry(&projects::Registry {
        projects: vec![temp.path().to_owned()],
        drafts,
        ..Default::default()
    })?;
    store.enqueue_prompt(
        &draft_target(&draft.id),
        Backend::Codex,
        temp.path(),
        None,
        crate::protocol::PromptMode::Normal,
        "fix this",
        &[],
    )?;
    assert_eq!(store.queued_prompts()?.len(), 1);
    assert_eq!(
        store.load_registry()?.drafts[0].harness,
        Some(Backend::Codex)
    );
    Ok(())
}

#[gpui::test]
fn untouched_drafts_prune_on_switch_and_quit(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::untouched_drafts_prune_on_switch_and_quit"
        ),
        cx,
        |cx, app, _, project| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let untouched = app.sessions.selected_draft.clone().expect("initial draft");
                    assert!(
                        crate::app::persistence::open()
                            .expect("store")
                            .load_drafts()
                            .expect("drafts")
                            .is_empty()
                    );
                    let snapshot = std::sync::Arc::make_mut(&mut app.snapshot);
                    std::sync::Arc::make_mut(&mut snapshot.conversation)
                        .push_transport_error("old session".into());
                    app.new_session(project.into(), window, cx);
                    assert!(
                        !app.sessions
                            .drafts
                            .iter()
                            .any(|draft| draft.id == untouched)
                    );
                    let engaged = app.sessions.selected_draft.clone().expect("next draft");
                    app.set_thinking_level(Some("high".into()), cx);
                    app.set_thinking_level(None, cx);
                    app.project.pending_trust_command = None;
                    app.overlays.view.project_trust = false;
                    app.new_session(project.into(), window, cx);
                    assert!(app.sessions.drafts.iter().any(|draft| draft.id == engaged));
                    let last = app.sessions.selected_draft.clone().expect("last draft");
                    let target = draft_target(&last);
                    assert!(app.sync_current_draft(&target));
                    assert!(app.sessions.selected_draft.is_none());
                    assert!(!app.sessions.drafts.iter().any(|draft| draft.id == last));
                    assert!(
                        app.sync_current_draft(&target),
                        "quit callback must not recreate the draft"
                    );
                    let saved = crate::app::persistence::open()
                        .expect("store")
                        .load_drafts()
                        .expect("drafts");
                    assert_eq!(saved.len(), 1);
                    assert_eq!(saved[0].id, engaged);
                    assert!(saved[0].app_session_id > 0);
                });
            });
        },
    );
}

#[gpui::test]
fn user_actions_save_empty_drafts(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(module_path!(), "::user_actions_save_empty_drafts"),
        cx,
        |cx, app, _, project| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    // Trust remains off so terminal/editor entry points record the request
                    // without launching a native process in this test.
                    app.project.repository.execution_allowed = false;
                    let model: crate::protocol::Model = serde_json::from_value(serde_json::json!({
                        "id": "test", "name": "Test", "provider": "test"
                    }))
                    .expect("model");
                    for action in [
                        "harness",
                        "model",
                        "effort",
                        "tier",
                        "sandbox",
                        "terminal",
                        "editor",
                        "attachment",
                        "workgraph",
                        "folder",
                    ] {
                        app.new_session(project.into(), window, cx);
                        let id = app.sessions.selected_draft.clone().expect("draft");
                        match action {
                            "harness" => app.change_draft_harness(Backend::Pi, window, cx),
                            "model" => {
                                let snapshot = std::sync::Arc::make_mut(&mut app.snapshot);
                                snapshot.harness = Some(Backend::Pi);
                                snapshot.access_mode = crate::runtime::HarnessAccessMode::Auto;
                                app.select_model_from_ui(&model, window, cx);
                            }
                            "effort" => app.set_thinking_level(Some("high".into()), cx),
                            "tier" => app.set_service_tier("fast".into(), cx),
                            "sandbox" => {
                                app.set_access_mode(crate::runtime::HarnessAccessMode::Full, cx)
                            }
                            "terminal" => {
                                app.activate_terminal_for_project(project.into(), window, cx)
                            }
                            "editor" => app.open_editor_request(
                                crate::app::workspace::editor::EditorRequest::Project(
                                    project.into(),
                                ),
                                window,
                                cx,
                            ),
                            "workgraph" => app.open_workgraph_surface(window, cx),
                            "folder" => {
                                app.sessions.folders.create("Work".into(), None);
                                let folder =
                                    app.sessions.folders.folders.last().expect("folder").id;
                                assert!(app.assign_session_folder(0, Some(folder), cx));
                                let session = app.sessions.draft_session_ids[&id];
                                assert_eq!(app.sessions.folders.folder_for(session), Some(folder));
                            }
                            "attachment" => {
                                let path = project.join("attachment.txt");
                                std::fs::write(&path, "attachment").expect("paste file");
                                app.composer.pastes.insert(
                                    draft_target(&id),
                                    vec![
                                        crate::app::composer::pastes::ComposerPaste::from_path(
                                            path,
                                        )
                                        .expect("paste"),
                                    ],
                                );
                                app.save_composer_attachments(&draft_target(&id));
                                app.remove_composer_paste(0, cx);
                            }
                            _ => unreachable!(),
                        }
                        app.project.pending_trust_command = None;
                        app.overlays.view.project_trust = false;
                        assert!(app.sync_current_draft(&draft_target(&id)), "{action}");
                        futures::executor::block_on(app.sessions.writer.flush())
                            .expect("engagement saved");
                        let saved = crate::app::persistence::open()
                            .expect("store")
                            .load_drafts()
                            .expect("drafts");
                        assert!(
                            saved
                                .iter()
                                .find(|draft| draft.id == id)
                                .expect(action)
                                .app_session_id
                                > 0,
                            "{action}"
                        );
                    }
                });
            });
        },
    );
}

#[gpui::test]
fn typing_then_clearing_a_draft_still_counts_as_engagement(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::typing_then_clearing_a_draft_still_counts_as_engagement"
        ),
        cx,
        |cx, app, _, project| {
            for value in [" ", ""] {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.composer.input.update(cx, |input, cx| {
                            input.set_value(value, window, cx);
                            cx.emit(gpui_component::input::InputEvent::Change);
                        });
                    });
                });
            }
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let id = app.sessions.selected_draft.clone().expect("draft");
                    app.new_session(project.into(), window, cx);
                    futures::executor::block_on(app.sessions.writer.flush()).expect("saved");
                    assert!(
                        crate::app::persistence::open()
                            .expect("store")
                            .load_drafts()
                            .expect("drafts")
                            .iter()
                            .find(|draft| draft.id == id)
                            .expect("saved draft")
                            .app_session_id
                            > 0
                    );
                });
            });
        },
    );
}

#[gpui::test]
fn failed_first_engagement_save_keeps_the_draft_and_retries_before_quit(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::failed_first_engagement_save_keeps_the_draft_and_retries_before_quit"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    let id = app.sessions.selected_draft.clone().expect("draft");
                    let mut store = crate::app::persistence::open().expect("store");
                    store.with_connection(|connection| connection.execute_batch(
                        "CREATE TRIGGER reject_draft BEFORE INSERT ON sessions BEGIN SELECT RAISE(FAIL,'fixture draft save failure'); END;"
                    )).expect("reject writes");
                    app.set_thinking_level(Some("high".into()), cx);
                    app.set_thinking_level(None, cx);
                    assert!(!app.sync_current_draft(&draft_target(&id)));
                    assert_eq!(app.sessions.selected_draft.as_deref(), Some(id.as_str()));
                    assert!(app.sessions.pending_draft_saves.contains(&id));
                    assert!(store.load_drafts().expect("drafts").is_empty());
                    store.with_connection(|connection| connection.execute_batch("DROP TRIGGER reject_draft;")).expect("allow writes");
                    assert!(app.sync_current_draft(&draft_target(&id)));
                    assert!(!app.sessions.pending_draft_saves.contains(&id));
                    assert!(store.load_drafts().expect("drafts").iter().any(|draft| draft.id == id));
                });
            });
        },
    );
}
