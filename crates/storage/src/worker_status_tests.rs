use super::*;

#[test]
fn worker_outcomes_survive_migration_rebind_and_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("state.sqlite3");
    let transcript = temp.path().join("worker.jsonl");
    std::fs::write(
        &transcript,
        format!(
            "{}\n{}\n",
            serde_json::json!({"type":"session","version":3,"id":"worker","cwd":temp.path(),"timestamp":"2026-11-18T00:00:00Z"}),
            serde_json::json!({"type":"message","id":"m1","parentId":null,"message":{"role":"assistant","content":[],"stopReason":"stop"}})
        ),
    )?;
    let family = agents::WorkerFamilyLink {
        project: temp.path().to_owned(),
        child_backend: Backend::Pi,
        parent_backend: Backend::Pi,
        child_session: transcript.to_string_lossy().into_owned(),
        parent_session: temp
            .path()
            .join("parent.jsonl")
            .to_string_lossy()
            .into_owned(),
        child_key: None,
        parent_key: None,
        execution: None,
        routing: None,
    };
    let store = StateStore::open_at(&path)?;
    store.save_worker_family(&family)?;
    let id = store.worker_outcomes()?.into_keys().next().unwrap();
    store.connection.execute_batch("ALTER TABLE worker_families DROP COLUMN last_status; UPDATE meta SET value='23' WHERE key='schema_version';")?;
    drop(store);
    let store = StateStore::open_at(&path)?;
    assert_eq!(store.worker_outcomes()?[&id], Some(AgentOutcome::Complete));
    for (status, expected) in [
        (WorkerStatus::Running, AgentOutcome::Incomplete),
        (WorkerStatus::Failed, AgentOutcome::Failed),
        (WorkerStatus::Idle, AgentOutcome::Complete),
        (WorkerStatus::Stopped, AgentOutcome::Incomplete),
    ] {
        store.save_worker_status(sessions::AppSessionId::new(id).unwrap(), status)?;
        store.save_worker_family(&family)?;
        assert_eq!(
            StateStore::open_at(&path)?.worker_outcomes()?[&id],
            Some(expected)
        );
    }
    Ok(())
}
