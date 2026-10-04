use super::*;
use crate::projects::ProjectList;
use crate::runtime::RunStatus;
use std::{sync::mpsc, time::Duration};

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
                    );
                    draft.app_session_id =
                        crate::app::session::draft_store::save(&draft).expect("saved draft");
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
                                | RuntimeCommand::SetAppSessionArchived { .. }
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
                                .insert("test".into(), RunStatus::Working);
                        }
                        app.request_application_quit(window, cx);
                        if active {
                            assert!(app.lifecycle.pending_quit.is_some());
                            app.confirm_application_quit(window, cx);
                        }
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

#[test]
fn recovery_keeps_other_writer_failures_and_unrelated_errors() {
    use PersistenceSource::{Composer, Session};
    let (mut writer, _) = SessionStateWriter::spawn(|| Err("unused".into()));
    let mut displayed = None;
    for composer_error in ["same error", "composer error"] {
        writer.update_error(Session, Err("same error".into()), &mut displayed);
        writer.update_error(Composer, Err(composer_error.into()), &mut displayed);
        writer.update_error(Composer, Ok(()), &mut displayed);
        assert_eq!(displayed.as_deref(), Some("same error"));
        writer.update_error(Session, Ok(()), &mut displayed);
        assert_eq!(displayed, None);
    }
    writer.update_error(Composer, Err("composer".into()), &mut displayed);
    displayed = Some("unrelated".into());
    writer.update_error(Composer, Ok(()), &mut displayed);
    assert_eq!(displayed.as_deref(), Some("unrelated"));
}

#[gpui::test]
fn failed_composer_save_blocks_quit(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(module_path!(), "::failed_composer_save_blocks_quit"),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let (writer, _) = crate::storage::ComposerPersistenceWorker::spawn(|| {
                        Err("fixture composer save failure".into())
                    });
                    app.composer.sessions = crate::sessions::ComposerSessions::new(
                        "test".into(),
                        vec![],
                        Box::new(writer),
                    );
                    app.composer
                        .sessions
                        .capture_current(crate::sessions::ComposerSnapshot::new(
                            "unsaved".into(),
                            0,
                            0..0,
                        ));
                    app.sessions.selected_draft = None;
                    app.request_application_quit(window, cx);
                    assert!(futures::executor::block_on(app.composer.sessions.flush()).is_err());
                });
            });
            cx.run_until_parked();
            cx.update(|_, cx| {
                let app = app.read(cx);
                assert!(!app.lifecycle.saving_before_quit);
                assert_eq!(
                    app.sessions.error.as_deref(),
                    Some("fixture composer save failure")
                );
            });
        },
    );
}

#[gpui::test]
fn quit_reflushes_composer_changes_queued_after_its_barrier(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::quit_reflushes_composer_changes_queued_after_its_barrier"
        ),
        cx,
        |cx, app, _, project| {
            let (started_tx, started_rx) = mpsc::channel();
            let (resume_tx, resume_rx) = mpsc::channel();
            let opens = std::sync::atomic::AtomicUsize::new(0);
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let (writer, _) = crate::storage::ComposerPersistenceWorker::spawn(move || {
                        if opens.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2 {
                            started_tx.send(()).unwrap();
                            resume_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                        }
                        persistence::shared()
                    });
                    let mut draft = crate::app::session::draft_store::new(
                        project.into(),
                        Some(crate::agents::Backend::Pi),
                        None,
                    );
                    draft.app_session_id = crate::app::session::draft_store::save(&draft)
                        .expect("persistent composer draft");
                    app.composer.sessions = crate::sessions::ComposerSessions::new(
                        crate::sessions::draft_target(&draft.id),
                        vec![],
                        Box::new(writer),
                    );
                    app.composer
                        .sessions
                        .capture_current(crate::sessions::ComposerSnapshot::new(
                            "initial".into(),
                            0,
                            0..0,
                        ));
                    app.sessions.selected_draft = None;
                    app.request_application_quit(window, cx);
                });
            });
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    let target = app.composer.sessions.current_target().to_owned();
                    app.composer
                        .sessions
                        .record_submission(&target, "late submission");
                })
            });
            resume_tx.send(()).unwrap();
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            cx.run_until_parked();
            cx.update(|_, cx| assert!(app.read(cx).lifecycle.saving_before_quit));
            resume_tx.send(()).unwrap();
            cx.update(|_, cx| {
                futures::executor::block_on(app.read(cx).composer.sessions.flush()).unwrap();
            });
            cx.run_until_parked();
            cx.update(|_, cx| assert!(!app.read(cx).lifecycle.saving_before_quit));
            assert!(
                persistence::open()
                    .unwrap()
                    .load_composer_sessions()
                    .unwrap()
                    .iter()
                    .any(|record| record.history.as_slice() == ["late submission"])
            );
        },
    );
}
