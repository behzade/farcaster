use super::*;

fn fixture() -> (tempfile::TempDir, SharedStateStore, DraftSession) {
    let temp = tempfile::tempdir().expect("fixture");
    let mut store =
        crate::storage::StateStore::open_at(&temp.path().join("state.sqlite3")).expect("store");
    let mut draft = DraftSession::with_id(
        Some(crate::agents::Backend::Pi),
        "draft".into(),
        temp.path().into(),
    );
    draft.app_session_id = store.allocate_app_session_id(&draft).expect("draft");
    (temp, SharedStateStore::new(store), draft)
}

#[test]
fn removal_after_an_in_flight_save_stays_deleted_and_latest_folders_win() {
    let (_temp, store, draft) = fixture();
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    let resume = std::sync::Mutex::new(Some(resume_rx));
    let worker_store = store.clone();
    let (writer, _) = SessionStateWriter::spawn(move || {
        if let Some(resume) = resume.lock().expect("gate").take() {
            started_tx.send(()).expect("started");
            resume.recv_timeout(Duration::from_secs(5)).expect("resume");
        }
        Ok(worker_store.clone())
    });
    writer.save_draft(draft.clone()).expect("save");
    started_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("in flight");
    writer.remove_draft(draft.id).expect("discard");
    let mut folders = SessionFolders::default();
    for index in 0..5 {
        folders.create(format!("Folder {index}"), None);
        writer.save_folders(folders.clone()).expect("folders");
    }
    resume_tx.send(()).expect("resume");
    futures::executor::block_on(writer.flush()).expect("drain");
    store
        .with(|store| {
            assert!(store.load_drafts()?.is_empty());
            assert_eq!(
                serde_json::to_value(store.load_session_folders()?).unwrap(),
                serde_json::to_value(&folders).unwrap()
            );
            Ok(())
        })
        .expect("verify");
}

#[test]
fn failed_batches_keep_deletions_for_retry_and_flush_reports_failure() {
    let (_temp, store, draft) = fixture();
    let blocked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let gate = blocked.clone();
    let worker_store = store.clone();
    let (writer, updates) = SessionStateWriter::spawn(move || {
        if gate.load(std::sync::atomic::Ordering::SeqCst) {
            Err("fixture unavailable store".into())
        } else {
            Ok(worker_store.clone())
        }
    });
    writer.remove_draft(draft.id).expect("remove");
    assert!(futures::executor::block_on(writer.flush()).is_err());
    assert!(updates.recv_blocking().expect("error").is_err());
    assert_eq!(
        store
            .with(|store| store.load_drafts())
            .expect("drafts")
            .len(),
        1
    );
    blocked.store(false, std::sync::atomic::Ordering::SeqCst);
    futures::executor::block_on(writer.flush()).expect("retry");
    assert!(
        store
            .with(|store| store.load_drafts())
            .expect("drafts")
            .is_empty()
    );
    assert!(updates.recv_blocking().expect("recovery").is_ok());
}

#[test]
fn shutdown_flush_drains_queued_changes() {
    let (_temp, store, mut draft) = fixture();
    let worker_store = store.clone();
    let (writer, _) = SessionStateWriter::spawn(move || Ok(worker_store.clone()));
    draft.title = Some("Saved before exit".into());
    writer.save_draft(draft).expect("save");
    futures::executor::block_on(writer.flush()).expect("saved before exit");
    drop(writer);
    assert_eq!(
        store.with(|store| store.load_drafts()).expect("drafts")[0]
            .title
            .as_deref(),
        Some("Saved before exit")
    );
}

#[gpui::test]
fn failed_handoffs_restore_drafts_and_folder_membership(cx: &mut gpui::TestAppContext) {
    use crate::{agents::Backend, app::composer::sessions::draft_target, runtime::RuntimeCommand};
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::failed_handoffs_restore_drafts_and_folder_membership"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    futures::executor::block_on(app.sessions.writer.flush())
                        .expect("initial saves");
                    let mut draft = crate::app::session::draft_store::new(
                        project.into(),
                        Some(Backend::Pi),
                        None,
                    )
                    .expect("draft");
                    app.sessions.selected_draft = Some(draft.id.clone());
                    app.switch_composer_target(draft_target(&draft.id), window, cx);
                    app.sessions.drafts.insert(0, draft.clone());
                    let failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
                    let gate = failed.clone();
                    let (writer, _) = SessionStateWriter::spawn(move || {
                        if gate.load(std::sync::atomic::Ordering::SeqCst) {
                            Err("fixture write failure".into())
                        } else {
                            persistence::shared()
                        }
                    });
                    app.sessions.writer = writer;
                    while runtime.try_recv_command().is_some() {}

                    app.change_draft_harness(Backend::Claude, window, cx);
                    assert_eq!(app.sessions.drafts[0], draft);
                    app.change_draft_project(project.join("other"), window, cx);
                    assert_eq!(app.sessions.drafts[0], draft);
                    assert_eq!(app.project.path, project);
                    // A submitted, materialized draft needs the archive command as
                    // well as its UI save. Failure must not leave it half archived.
                    draft.submitted = true;
                    draft.session_path = Some(project.join("session"));
                    app.sessions.drafts[0] = draft.clone();
                    app.request_draft_archive(draft.id.clone(), true, window, cx);
                    assert_eq!(app.sessions.drafts[0], draft);

                    app.sessions.folders.create("Folder".into(), None);
                    let folder = app.sessions.folders.folders.last().unwrap().id;
                    app.move_session_to_folder(
                        draft.app_session_id,
                        project.join("session"),
                        crate::sessions::FolderDestination::Folder(folder),
                        true,
                        window,
                        cx,
                    );
                    assert_eq!(app.sessions.folders.folder_for(draft.app_session_id), None);
                    while let Some(command) = runtime.try_recv_command() {
                        assert!(!matches!(
                            command,
                            RuntimeCommand::NewSession { .. }
                                | RuntimeCommand::SetSessionArchived { .. }
                        ));
                    }
                    failed.store(false, std::sync::atomic::Ordering::SeqCst);
                    futures::executor::block_on(app.sessions.writer.flush())
                        .expect("retry rollback");
                    let store = persistence::open().expect("store");
                    assert!(
                        !store
                            .load_drafts()
                            .expect("drafts")
                            .iter()
                            .find(|saved| saved.id == draft.id)
                            .unwrap()
                            .archived
                    );
                    assert_eq!(
                        store
                            .load_session_folders()
                            .expect("folders")
                            .folder_for(draft.app_session_id),
                        None
                    );
                })
            });
        },
    );
}

#[test]
fn flush_waits_without_blocking_its_caller() {
    use futures::FutureExt;
    let (_temp, store, draft) = fixture();
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    let resume = std::sync::Mutex::new(Some(resume_rx));
    let (writer, _) = SessionStateWriter::spawn(move || {
        if let Some(resume) = resume.lock().unwrap().take() {
            started_tx.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        Ok(store.clone())
    });
    writer.save_draft(draft).unwrap();
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut flush = Box::pin(writer.flush());
    assert!(flush.as_mut().now_or_never().is_none());
    resume_tx.send(()).unwrap();
    futures::executor::block_on(flush).expect("saved");
}

#[gpui::test]
fn failed_save_blocks_normal_and_confirmed_quit(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::failed_save_blocks_normal_and_confirmed_quit"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    futures::executor::block_on(app.sessions.writer.flush()).unwrap();
                    let (writer, _) =
                        SessionStateWriter::spawn(|| Err("fixture quit save failure".into()));
                    app.sessions.writer = writer;
                    app.sessions
                        .writer
                        .save_projects(ProjectList::default())
                        .unwrap();
                });
            });
            for active in [false, true] {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.sessions.error = None;
                        if active {
                            app.activity
                                .run_statuses
                                .insert("test".into(), "Working".into());
                        }
                        app.request_application_quit(window, cx);
                        if active {
                            assert!(app.lifecycle.pending_quit.is_some());
                            app.confirm_application_quit(window, cx);
                        }
                        // Wait for the worker, then let the UI consume the quit result.
                        assert!(futures::executor::block_on(app.sessions.writer.flush()).is_err());
                    })
                });
                cx.run_until_parked();
                cx.update(|_, cx| assert!(!app.read(cx).lifecycle.saving_before_quit));
                cx.update(|_, cx| {
                    assert_eq!(
                        app.read(cx).sessions.error.as_deref(),
                        Some("fixture quit save failure")
                    )
                });
            }
        },
    );
}

#[gpui::test]
fn quit_waits_for_changes_queued_after_its_first_flush(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::quit_waits_for_changes_queued_after_its_first_flush"
        ),
        cx,
        |cx, app, _, _| {
            let (started_tx, started_rx) = mpsc::channel();
            let (resume_tx, resume_rx) = mpsc::channel();
            let opens = std::sync::atomic::AtomicUsize::new(0);
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    futures::executor::block_on(app.sessions.writer.flush()).unwrap();
                    let (writer, _) = SessionStateWriter::spawn(move || {
                        if opens.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2 {
                            started_tx.send(()).unwrap();
                            resume_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                        }
                        persistence::shared()
                    });
                    app.sessions.writer = writer;
                    app.sessions.selected_draft = None;
                    app.sessions
                        .writer
                        .save_projects(ProjectList::default())
                        .unwrap();
                    app.request_application_quit(window, cx);
                });
            });
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    let mut folders = app.sessions.folders.clone();
                    folders.create("Queued during quit".into(), None);
                    app.sessions.writer.save_folders(folders).unwrap();
                })
            });
            resume_tx.send(()).unwrap();
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            cx.run_until_parked();
            cx.update(|_, cx| {
                assert!(
                    app.read(cx).lifecycle.saving_before_quit,
                    "quit did not wait for the later change"
                )
            });
            resume_tx.send(()).unwrap();
            cx.update(|_, cx| {
                futures::executor::block_on(app.read(cx).sessions.writer.flush()).unwrap();
            });
            cx.run_until_parked();
            cx.update(|_, cx| {
                assert!(
                    !app.read(cx).lifecycle.saving_before_quit,
                    "saved state should allow quit"
                )
            });
            assert!(
                persistence::open()
                    .unwrap()
                    .load_session_folders()
                    .unwrap()
                    .folders
                    .iter()
                    .any(|folder| folder.name == "Queued during quit")
            );
        },
    );
}
