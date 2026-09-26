use std::{error::Error, path::Path};

use rusqlite::{Connection, params};

use super::relocate_snapshot_session_locators;

#[test]
fn legacy_snapshot_migrates_then_relocates_without_changing_source() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("private");
    std::fs::create_dir(&source)?;
    std::fs::create_dir(&destination)?;
    let source_db = source.join("state.sqlite3");
    let destination_db = destination.join("state.sqlite3");
    let old_locator = source.join("session-locators/codex-cli/legacy-child");
    let new_locator = destination.join("session-locators/codex-cli/legacy-child");
    let mut legacy = Connection::open(&source_db)?;
    let transaction = legacy.transaction()?;
    crate::migrate_legacy::migrate_to_v11(&transaction, 1)?;
    transaction.commit()?;
    legacy.execute("INSERT INTO projects VALUES('/native/project',1)", [])?;
    legacy.execute(
        "INSERT INTO sessions(path,id,project,title,first_user_message,timestamp,parent_session,
         modified_ms,file_size,message_count,input_tokens,output_tokens,cache_read_tokens,
         cache_write_tokens,total_tokens,cost_micros,search_text,settled_ms,app_session_id,harness)
         VALUES(?1,'legacy-native','/native/project','legacy title','hello','',NULL,
         10,0,1,0,0,0,0,0,0,'hello',123,41,'codex-cli')",
        [old_locator.to_str()],
    )?;
    legacy.execute(
        "INSERT INTO app_sessions(id,session_path,created_ms,harness) VALUES(41,?1,10,'codex-cli')",
        [old_locator.to_str()],
    )?;
    legacy.execute(
        "INSERT INTO composer_sessions VALUES(?1,'legacy draft',0,0,0,'[]',10)",
        [format!("session:{}", old_locator.display())],
    )?;
    legacy.execute(
        "INSERT INTO outbox(target,project,session_path,mode,message,created_ms,harness)
         VALUES(?1,'/native/project',?2,'normal','queued legacy',10,'codex-cli')",
        params![
            format!("session:{}", old_locator.display()),
            old_locator.to_str()
        ],
    )?;
    drop(legacy);
    let original = std::fs::read(&source_db)?;
    crate::snapshot_database(&source_db, &destination_db)?;
    assert_eq!(
        relocate_snapshot_session_locators(&destination_db, &source, &destination)?,
        1
    );
    let mut store = crate::StateStore::open_at(&destination_db)?;
    let (version, count): (String, i64) = store.with_connection(|connection| {
        connection.query_row(
            "SELECT value,(SELECT COUNT(*) FROM sessions) FROM meta WHERE key='schema_version'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    })?;
    assert_eq!(version, crate::SCHEMA_VERSION.to_string());
    assert_eq!(count, 1);
    let migrated: (i64, String, i64) = store.with_connection(|connection| {
        connection.query_row(
            "SELECT id,backend_id,archived_at FROM sessions WHERE locator=?1",
            [new_locator.to_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
    })?;
    assert_eq!(migrated, (41, "legacy-native".into(), 123));
    let related: (String, String) = store.with_connection(|connection| {
        connection.query_row(
            "SELECT c.text,o.message FROM composer_sessions c JOIN outbox o USING(session_id)
         WHERE c.session_id=41",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    })?;
    assert_eq!(related, ("legacy draft".into(), "queued legacy".into()));
    assert_eq!(std::fs::read(&source_db)?, original);
    let source_connection = Connection::open(&source_db)?;
    let source_version: String = source_connection.query_row(
        "SELECT value FROM meta WHERE key='schema_version'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(source_version, "11");
    let source_path: String =
        source_connection.query_row("SELECT path FROM sessions", [], |row| row.get(0))?;
    assert_eq!(source_path, old_locator.to_string_lossy());
    Ok(())
}

#[test]
fn relocation_rejects_unsupported_legacy_schema() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let database = root.path().join("state.sqlite3");
    let connection = Connection::open(&database)?;
    connection.execute_batch(
        "CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);
         INSERT INTO meta VALUES('schema_version','99');
         CREATE TABLE sessions(path TEXT PRIMARY KEY,id TEXT);",
    )?;
    assert!(
        relocate_snapshot_session_locators(&database, root.path(), &root.path().join("private"))
            .is_err()
    );
    Ok(())
}

fn locator(connection: &Connection, id: i64) -> rusqlite::Result<Option<String>> {
    connection.query_row("SELECT locator FROM sessions WHERE id=?1", [id], |row| {
        row.get(0)
    })
}

fn fixture(database: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open(database)?;
    connection.execute_batch(include_str!("schema.sql"))?;
    connection.execute(
        "INSERT INTO meta(key,value) VALUES('schema_version',?1)",
        [crate::SCHEMA_VERSION.to_string()],
    )?;
    connection
        .execute_batch("INSERT INTO projects(id,path,added_ms) VALUES(7,'/native/project',1);")?;
    Ok(connection)
}

#[test]
fn relocation_preserves_profile_archive_and_related_rows() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let database = root.path().join("state.sqlite3");
    let source = root.path().join("source");
    let destination = root.path().join("private");
    let connection = fixture(&database)?;
    let suffix = "profiles/0123456789abcdef0123456789abcdef/project-hash/codex-cli/child%2Fid";
    let parent_locator = source.join("session-locators/project-hash/codex-cli/parent");
    let child_locator = source.join("session-locators").join(suffix);
    let native_locator = root.path().join("native/pi/session.jsonl");
    let near_prefix = source.join("session-locators-other/codex-cli/session");
    let escaped = source.join("session-locators/../outside/session");
    for (id, path) in [
        (41, &parent_locator),
        (42, &child_locator),
        (43, &native_locator),
        (44, &near_prefix),
        (45, &escaped),
    ] {
        connection.execute(
            "INSERT INTO sessions(id,project_id,harness,locator,modified_ms,created_ms)
             VALUES(?1,7,'codex-cli',?2,100,90)",
            params![id, path.to_str()],
        )?;
    }
    connection.execute_batch(
        "UPDATE sessions SET backend_id='parent-native' WHERE id=41;
         UPDATE sessions SET profile_id='0123456789abcdef0123456789abcdef', backend_id='child/id',
             parent_id=41, parent_backend_id='parent-native', archived_at=123, client_key='draft-child'
             WHERE id=42;
         INSERT INTO sessions(id,project_id,harness,modified_ms,created_ms) VALUES(46,7,'codex-cli',100,90);
         INSERT INTO worker_families(child_id,execution_json,routing_json) VALUES(42,'{}','{}');
         INSERT INTO session_models(session_id,model) VALUES(42,'fixture-model');
         INSERT INTO composer_sessions(session_id,text,cursor,selection_start,selection_end,history_json,updated_ms)
             VALUES(42,'draft',5,5,5,'[]',100);
         INSERT INTO session_events(session_id,seq,t,schema_version,body) VALUES(42,1,100,1,'{}');
         INSERT INTO outbox(session_id,mode,message,created_ms) VALUES(42,'normal','queued',100);",
    )?;
    drop(connection);

    assert_eq!(
        relocate_snapshot_session_locators(&database, &source, &destination)?,
        2
    );
    let connection = Connection::open(&database)?;
    assert_eq!(
        locator(&connection, 41)?,
        destination
            .join("session-locators/project-hash/codex-cli/parent")
            .to_str()
            .map(str::to_owned)
    );
    assert_eq!(
        locator(&connection, 42)?,
        destination
            .join("session-locators")
            .join(suffix)
            .to_str()
            .map(str::to_owned)
    );
    for (id, path) in [(43, &native_locator), (44, &near_prefix), (45, &escaped)] {
        assert_eq!(locator(&connection, id)?, path.to_str().map(str::to_owned));
    }
    assert_eq!(locator(&connection, 46)?, None);
    let saved: (i64, String, String, i64, String, i64, String) = connection.query_row(
        "SELECT project_id,profile_id,backend_id,parent_id,parent_backend_id,archived_at,client_key
         FROM sessions WHERE id=42",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        },
    )?;
    assert_eq!(
        saved,
        (
            7,
            "0123456789abcdef0123456789abcdef".into(),
            "child/id".into(),
            41,
            "parent-native".into(),
            123,
            "draft-child".into()
        )
    );
    let related: (String, String, String, String, String) = connection.query_row(
        "SELECT m.model,c.text,e.body,o.message,f.execution_json FROM session_models m
         JOIN composer_sessions c USING(session_id) JOIN session_events e USING(session_id)
         JOIN outbox o USING(session_id) JOIN worker_families f ON f.child_id=m.session_id
         WHERE m.session_id=42",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    assert_eq!(
        related,
        (
            "fixture-model".into(),
            "draft".into(),
            "{}".into(),
            "queued".into(),
            "{}".into()
        )
    );
    assert!(!connection.prepare("PRAGMA foreign_key_check")?.exists([])?);
    assert_eq!(
        relocate_snapshot_session_locators(&database, &source, &destination)?,
        0
    );
    Ok(())
}

#[test]
fn relocation_conflict_rolls_back_every_change() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let database = root.path().join("state.sqlite3");
    let source = root.path().join("source");
    let destination = root.path().join("private");
    let connection = fixture(&database)?;
    let first = source.join("session-locators/codex-cli/first");
    let second = source.join("session-locators/codex-cli/second");
    let conflicting = destination.join("session-locators/codex-cli/second");
    for (id, path) in [(1, &first), (2, &second), (3, &conflicting)] {
        connection.execute(
            "INSERT INTO sessions(id,project_id,harness,locator,modified_ms,created_ms)
             VALUES(?1,7,'codex-cli',?2,1,1)",
            params![id, path.to_str()],
        )?;
    }
    assert!(relocate_snapshot_session_locators(&database, &source, &destination).is_err());
    assert_eq!(locator(&connection, 1)?, first.to_str().map(str::to_owned));
    assert_eq!(locator(&connection, 2)?, second.to_str().map(str::to_owned));
    let missing = root.path().join("missing.sqlite3");
    assert!(relocate_snapshot_session_locators(&missing, &source, &destination).is_err());
    assert!(!missing.exists());
    Ok(())
}
