use super::*;

#[test]
fn account_metadata_survives_restart_without_credentials() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("state.sqlite3");
    let mut snapshot = agents::accounts::AccountsSnapshot::default();
    snapshot.accounts.push(agents::accounts::Account {
        id: "local-id".into(),
        identity: agents::accounts::AccountIdentity {
            authority: "anthropic".into(),
            email: Some("me@example.com".into()),
            ..Default::default()
        },
    });
    StateStore::open_at(&path)
        .expect("store")
        .save_accounts(&snapshot)
        .expect("save");
    let loaded = StateStore::open_at(&path)
        .expect("reopen")
        .load_accounts()
        .expect("load");
    assert_eq!(loaded, snapshot);
}

#[test]
fn accounts_migrate_v22_without_losing_settings() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = StateStore::open_at(&path).unwrap();
    store
        .connection
        .execute_batch(
            "DROP TABLE account_state; UPDATE meta SET value='22' WHERE key='schema_version';",
        )
        .unwrap();
    drop(store);
    let store = StateStore::open_at(&path).unwrap();
    assert_eq!(
        store.load_accounts().unwrap(),
        agents::accounts::AccountsSnapshot::default()
    );
    let version: i64 = store
        .connection
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
}
