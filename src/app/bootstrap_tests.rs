use super::*;

use std::time::SystemTime;

fn remembered_session(project: &Path, id: &str, title: &str) -> SessionSummary {
    SessionSummary::from_cached(
        id.into(),
        project.join(format!("{id}.jsonl")),
        project.to_path_buf(),
        title.into(),
        "first prompt".into(),
        "2026-09-24T00:00:00Z".into(),
        None,
        SystemTime::now(),
        3,
        crate::sessions::UsageSummary::default(),
        false,
        false,
        title.into(),
    )
}

#[gpui::test]
fn startup_restores_selected_chat_and_its_unsent_composer(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_prepared_offline_app(
        concat!(
            module_path!(),
            "::startup_restores_selected_chat_and_its_unsent_composer"
        ),
        cx,
        |project| {
            let project_path = project.canonicalize().unwrap();
            let project = project_path.as_path();
            let other = project.join("other");
            std::fs::create_dir(&other).unwrap();
            let selected = remembered_session(&other, "selected", "Selected chat");
            let mut newer = remembered_session(&other, "newer", "Background update");
            newer.modified += std::time::Duration::from_secs(30);
            let mut store = crate::app::persistence::open().unwrap();
            store
                .replace_sessions(&[
                    remembered_session(project, "first", "First project"),
                    selected.clone(),
                    newer,
                ])
                .unwrap();
            store
                .save_composer_session(&crate::storage::ComposerRecord {
                    target: session_target(&selected.path),
                    text: "continue here".into(),
                    cursor: 13,
                    attachments: vec![crate::storage::ComposerAttachment::TextFile {
                        path: project.join("notes.txt"),
                    }],
                    ..Default::default()
                })
                .unwrap();
        },
        |cx, app, _, project| {
            let project_path = project.canonicalize().unwrap();
            let project = project_path.as_path();
            let other = project.join("other");
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.select_session(other.join("selected.jsonl"), other.clone(), window, cx);
                    futures::executor::block_on(app.sessions.writer.flush()).unwrap();
                    let launched =
                        crate::app::infrastructure::launch::resolve_project(None).unwrap();
                    assert_eq!(
                        launched, other,
                        "normal launch must return to the selected project"
                    );
                    let restored = persisted::load(&launched, None, cx);
                    assert_eq!(
                        restored.initial_session.as_ref().unwrap().path,
                        other.join("selected.jsonl")
                    );
                    assert_eq!(restored.composer_sessions.current().text, "continue here");
                    assert_eq!(
                        restored
                            .composer_sessions
                            .saved_attachments()
                            .next()
                            .unwrap()
                            .1
                            .len(),
                        1
                    );
                    assert!(
                        restored.drafts.is_empty(),
                        "restoring a chat must not add an empty draft"
                    );
                    let explicit =
                        crate::app::infrastructure::launch::resolve_project(Some(project.into()))
                            .unwrap();
                    assert_eq!(explicit, project);
                    let other = persisted::load(&explicit, None, cx);
                    assert!(
                        other.initial_session.is_none(),
                        "explicit project must not restore another project's chat"
                    );
                    assert_eq!(other.initial_draft.project, project);
                })
            });
        },
    );
}

#[gpui::test]
fn startup_restores_a_draft_then_opens_its_bound_history(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::startup_restores_a_draft_then_opens_its_bound_history"
        ),
        cx,
        |cx, app, _, project| {
            let project_path = project.canonicalize().unwrap();
            let project = project_path.as_path();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let id = app.sessions.selected_draft.clone().unwrap();
                    let target = draft_target(&id);
                    app.composer.sessions =
                        crate::app::composer::sessions::load(target.clone(), cx).0;
                    app.composer.input.update(cx, |input, cx| {
                        input.set_value("unfinished request", window, cx)
                    });
                    app.capture_composer_session(cx);
                    assert!(app.sync_current_draft(&target));
                    app.remember_session_selection();
                    futures::executor::block_on(app.sessions.writer.flush()).unwrap();
                    futures::executor::block_on(app.composer.sessions.flush()).unwrap();
                    let restored = persisted::load(project, None, cx);
                    assert_eq!(restored.initial_draft.id, id);
                    assert!(restored.initial_session.is_none());
                    assert_eq!(
                        restored.composer_sessions.current().text,
                        "unfinished request"
                    );
                    assert_eq!(restored.drafts.len(), 1);

                    let mut bound = restored.initial_draft;
                    bound.submitted = true;
                    bound.session_path = Some(project.join("bound.jsonl"));
                    {
                        let mut store = crate::app::persistence::open().unwrap();
                        store.allocate_app_session_id(&bound).unwrap();
                    }
                    let restored = persisted::load(project, None, cx);
                    assert_eq!(
                        restored.initial_session.as_ref().unwrap().path,
                        project.join("bound.jsonl")
                    );
                    assert_eq!(restored.initial_draft.id, id);
                    assert_eq!(
                        restored.composer_sessions.current_target(),
                        session_target(&project.join("bound.jsonl"))
                    );
                    assert!(restored.drafts.is_empty());
                    assert_eq!(
                        restored.composer_sessions.current().text,
                        "unfinished request"
                    );
                })
            });
        },
    );
}

#[gpui::test]
fn the_rail_paints_stored_chats_before_the_runtime_answers(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_prepared_offline_app(
        concat!(
            module_path!(),
            "::the_rail_paints_stored_chats_before_the_runtime_answers"
        ),
        cx,
        |project| {
            let session = remembered_session(project, "remembered", "Remembered chat");
            std::fs::write(&session.path, "{}").expect("write session file");
            let mut store = crate::app::persistence::open().expect("open state");
            store
                .replace_sessions(&[session])
                .expect("index the stored session");
        },
        |cx, app, _, _| {
            cx.update(|_, cx| {
                let app = app.read(cx);
                assert_eq!(
                    app.sessions.all.len(),
                    1,
                    "rail catalog came from the store"
                );
                assert_eq!(app.sessions.visible.len(), 1);
                assert_eq!(app.sessions.all[0].title, "Remembered chat");
            });
        },
    );
}

#[test]
fn warming_uses_the_sessions_profile_and_rejects_a_removed_profile() {
    let directory = tempfile::tempdir().expect("project");
    let project = directory.path();
    let profile = crate::agents::HarnessProfile {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Warm history".into(),
        backend: Backend::Pi,
        executable: PathBuf::from("pi"),
        data_directory: None,
    };
    let config = crate::agents::AgentLaunchConfig::default();
    config
        .profiles
        .replace(vec![profile.clone()])
        .expect("profile");
    let mut session = remembered_session(project, "recent", "Recent chat");
    let path = project.join("recent.jsonl");
    session.profile_id = Some(profile.id.clone());
    std::fs::write(
        &path,
        concat!(
            "{\"type\":\"session\",\"id\":\"recent\",\"cwd\":\"/project\"}\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n",
        ),
    )
    .expect("history");
    session.path = path;
    let sessions = [session];
    let warmed = warm_recent_history(&sessions, project, config.clone())
        .expect("warm task")
        .join()
        .expect("warm thread")
        .expect("profile history");
    assert!(warmed);
    config.profiles.replace(vec![]).expect("remove profile");
    let error = warm_recent_history(&sessions, project, config)
        .expect("warm task")
        .join()
        .expect("warm thread")
        .expect_err("removed profile");
    assert!(error.contains("unknown harness profile"), "{error}");
}

#[test]
fn warming_uses_catalog_roots_for_app_children_and_orphans() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path();
    let mut root = remembered_session(project, "root", "Root");
    root.app_session_id = 1;
    std::fs::write(
        &root.path,
        "{\"type\":\"session\",\"id\":\"root\",\"cwd\":\"/project\"}\n",
    )
    .unwrap();
    let mut child = remembered_session(project, "child", "Child");
    child.app_session_id = 2;
    child.parent_app_session_id = Some(root.app_session_id);
    child.modified = root.modified + std::time::Duration::from_secs(1);
    let sessions = [root.clone(), child];
    assert!(
        warm_recent_history(&sessions, project, Default::default())
            .unwrap()
            .join()
            .unwrap()
            .unwrap()
    );

    root.parent_session = Some("missing-parent".into());
    assert!(
        warm_recent_history(&[root], project, Default::default())
            .expect("an orphan remains a visible catalog root")
            .join()
            .unwrap()
            .unwrap()
    );
}
