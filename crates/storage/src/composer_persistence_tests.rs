use super::*;
use crate::agents::Backend;
use crate::sessions::session_target;

#[test]
fn bound_draft_aliases_preserve_write_and_delete_order() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let mut store = StateStore::open_at(&temp.path().join("gui.sqlite3"))?;
    let mut draft = crate::sessions::DraftSession::new(
        Some(Backend::Pi),
        "draft".into(),
        0,
        temp.path().to_path_buf(),
        1,
    );
    draft.session_path = Some(temp.path().join("session.jsonl"));
    store.allocate_app_session_id(&draft)?;
    let bound = session_target(
        draft
            .session_path
            .as_ref()
            .expect("test operation should succeed"),
    );
    let save = |target: String, text: &str| {
        PersistenceCommand::Save(ComposerRecord {
            target,
            text: text.into(),
            ..ComposerRecord::default()
        })
    };
    let mut pending = vec![
        save("draft:draft".into(), "older"),
        save(bound.clone(), "newer"),
    ];
    flush(&store, &mut pending)?;
    assert!(pending.is_empty());
    assert_eq!(store.load_composer_sessions()?[0].text, "newer");
    pending.extend([
        save(bound, "obsolete"),
        PersistenceCommand::Delete("draft:draft".into()),
    ]);
    flush(&store, &mut pending)?;
    assert!(store.load_composer_sessions()?.is_empty());
    Ok(())
}

#[test]
fn a_partial_failure_retries_only_the_uncommitted_suffix() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = tempfile::tempdir()?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    for id in ["first", "second"] {
        let draft =
            sessions::DraftSession::with_id(Some(Backend::Pi), id.into(), temp.path().into());
        store.allocate_app_session_id(&draft)?;
    }
    store.connection.execute_batch(
        "CREATE TRIGGER reject_composer BEFORE INSERT ON composer_sessions
         WHEN NEW.text='fail' BEGIN SELECT RAISE(FAIL, 'fixture failure'); END;",
    )?;
    let record = |target: &str, text: &str| ComposerRecord {
        target: format!("draft:{target}"),
        text: text.into(),
        ..Default::default()
    };
    let mut pending = vec![
        PersistenceCommand::Save(record("first", "saved")),
        PersistenceCommand::Save(record("second", "fail")),
        PersistenceCommand::Delete("draft:first".into()),
    ];
    assert!(flush(&store, &mut pending).is_err());
    assert_eq!(pending.len(), 2);
    assert_eq!(store.load_composer_sessions()?[0].text, "saved");
    store
        .connection
        .execute_batch("DROP TRIGGER reject_composer;")?;
    flush(&store, &mut pending)?;
    assert!(pending.is_empty());
    let records = store.load_composer_sessions()?;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].target, "draft:second");
    Ok(())
}

#[test]
fn flush_reports_open_failure_then_retries_latest_save_and_delete()
-> Result<(), Box<dyn std::error::Error>> {
    use sessions::ComposerPersistence;
    use std::sync::atomic::{AtomicBool, Ordering};
    let temp = tempfile::tempdir()?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    for id in ["keep", "delete"] {
        let draft =
            sessions::DraftSession::with_id(Some(Backend::Pi), id.into(), temp.path().into());
        store.allocate_app_session_id(&draft)?;
        store.save_composer_session(&ComposerRecord {
            target: format!("draft:{id}"),
            text: "initial".into(),
            ..Default::default()
        })?;
    }
    let store = SharedStateStore::new(store);
    let worker_store = store.clone();
    let blocked = Arc::new(AtomicBool::new(true));
    let gate = blocked.clone();
    let (writer, updates) = ComposerPersistenceWorker::spawn(move || {
        if gate.load(Ordering::SeqCst) {
            Err("fixture unavailable".into())
        } else {
            Ok(worker_store.clone())
        }
    });
    let save = |text: &str| {
        writer.save(ComposerRecord {
            target: "draft:keep".into(),
            text: text.into(),
            ..Default::default()
        })
    };
    save("old");
    writer.delete("draft:delete".into());
    assert!(futures::executor::block_on(writer.flush()).is_err());
    assert!(updates.recv_blocking()?.is_err());
    assert!(futures::executor::block_on(writer.flush()).is_err());
    assert!(updates.try_recv().is_err(), "repeated errors are coalesced");
    save("latest");
    assert_eq!(writer.revision(), 3);
    blocked.store(false, Ordering::SeqCst);
    futures::executor::block_on(writer.flush())?;
    assert!(updates.recv_blocking()?.is_ok());
    let records = store.with(|store| store.load_composer_sessions())?;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].text, "latest");
    Ok(())
}

#[test]
fn flush_is_nonblocking_and_stops_before_later_commands() -> Result<(), Box<dyn std::error::Error>>
{
    use futures::FutureExt;
    use sessions::ComposerPersistence;
    let temp = tempfile::tempdir()?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let draft =
        sessions::DraftSession::with_id(Some(Backend::Pi), "draft".into(), temp.path().into());
    store.allocate_app_session_id(&draft)?;
    let store = SharedStateStore::new(store);
    let worker_store = store.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let (writer, _) = ComposerPersistenceWorker::spawn(move || {
        started_tx.send(()).expect("started");
        if resume_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("resume")
        {
            Ok(worker_store.clone())
        } else {
            Err("later write failed".into())
        }
    });
    writer.save(ComposerRecord {
        target: "draft:draft".into(),
        text: "before barrier".into(),
        ..Default::default()
    });
    let mut barrier = writer.flush();
    started_rx.recv_timeout(Duration::from_secs(5))?;
    assert!(barrier.as_mut().now_or_never().is_none());
    writer.delete("draft:draft".into());
    let later = writer.flush();
    resume_tx.send(true)?;
    futures::executor::block_on(barrier)?;
    started_rx.recv_timeout(Duration::from_secs(5))?;
    resume_tx.send(false)?;
    assert!(futures::executor::block_on(later).is_err());
    assert_eq!(
        store.with(|store| store.load_composer_sessions())?[0].text,
        "before barrier"
    );
    resume_tx.send(true)?;
    drop(writer);
    assert!(
        store
            .with(|store| store.load_composer_sessions())?
            .is_empty()
    );
    Ok(())
}

#[test]
fn a_stopped_writer_reports_failed_sends_and_flush() {
    use sessions::ComposerPersistence;
    let (sender, receiver) = mpsc::channel();
    drop(receiver);
    let (updates, errors) = async_channel::unbounded();
    let writer = ComposerPersistenceWorker {
        sender,
        updates,
        revision: Cell::new(0),
        worker: None,
    };
    writer.save(ComposerRecord::default());
    assert!(errors.recv_blocking().expect("send error").is_err());
    assert_eq!(writer.revision(), 0);
    assert!(futures::executor::block_on(writer.flush()).is_err());
}
