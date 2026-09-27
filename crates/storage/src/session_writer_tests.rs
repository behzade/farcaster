use super::*;

fn fixture() -> (tempfile::TempDir, SharedStateStore, DraftSession) {
    let temp = tempfile::tempdir().expect("fixture");
    let mut store = crate::StateStore::open_at(&temp.path().join("state.sqlite3")).expect("store");
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
    let (writer, updates) = SessionStateWriter::spawn(move || Ok(worker_store.clone()));
    draft.title = Some("Saved before exit".into());
    writer.save_draft(draft).expect("save");
    drop(writer);
    assert!(
        updates.recv_blocking().is_err(),
        "shutdown saved without errors"
    );
    assert_eq!(
        store.with(|store| store.load_drafts()).expect("drafts")[0]
            .title
            .as_deref(),
        Some("Saved before exit")
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

#[test]
fn a_stopped_writer_rejects_changes_and_flush() {
    let (sender, receiver) = mpsc::channel();
    drop(receiver);
    let writer = SessionStateWriter {
        sender,
        revision: Cell::new(0),
    };
    assert!(writer.save_projects(ProjectList::default()).is_err());
    assert_eq!(writer.revision(), 0);
    assert!(futures::executor::block_on(writer.flush()).is_err());
}
