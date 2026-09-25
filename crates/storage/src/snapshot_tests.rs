use std::{fs, path::Path};

use rusqlite::Connection;

use super::snapshot_database;

fn values(connection: &Connection) -> rusqlite::Result<Vec<String>> {
    connection
        .prepare("SELECT value FROM fixture ORDER BY id")?
        .query_map([], |row| row.get(0))?
        .collect()
}

fn fixture(path: &Path, wal: bool) -> rusqlite::Result<Connection> {
    let connection = Connection::open(path)?;
    if wal {
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")?;
    }
    connection.execute_batch(
        "CREATE TABLE fixture(id INTEGER PRIMARY KEY, value TEXT);
         INSERT INTO fixture VALUES(1, 'committed');
         CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
         INSERT INTO meta VALUES('schema_version', '1');",
    )?;
    Ok(connection)
}

#[test]
fn snapshot_keeps_live_wal_commits_and_excludes_pending_writes() -> Result<(), String> {
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let source = root.path().join("source.sqlite3");
    let destination = root.path().join("snapshot.sqlite3");
    let writer = fixture(&source, true).map_err(|error| error.to_string())?;
    writer
        .execute_batch("BEGIN IMMEDIATE; INSERT INTO fixture VALUES(2, 'pending');")
        .map_err(|error| error.to_string())?;
    let database_before = fs::read(&source).map_err(|error| error.to_string())?;
    let wal_before =
        fs::read(root.path().join("source.sqlite3-wal")).map_err(|error| error.to_string())?;

    snapshot_database(&source, &destination)?;
    let copy = Connection::open(&destination).map_err(|error| error.to_string())?;
    assert_eq!(
        values(&copy).map_err(|error| error.to_string())?,
        ["committed"]
    );
    let version: String = copy
        .query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(version, "1");
    copy.execute("INSERT INTO fixture VALUES(3, 'copy only')", [])
        .map_err(|error| error.to_string())?;
    assert_eq!(
        values(&writer).map_err(|error| error.to_string())?,
        ["committed", "pending"]
    );
    assert_eq!(
        fs::read(&source).map_err(|error| error.to_string())?,
        database_before
    );
    assert_eq!(
        fs::read(root.path().join("source.sqlite3-wal")).map_err(|error| error.to_string())?,
        wal_before
    );
    writer
        .execute_batch("ROLLBACK")
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn snapshot_handles_escaped_paths_without_changing_source() -> Result<(), String> {
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let source = root.path().join("source ?#%.sqlite3");
    let destination = root.path().join("snapshot '?.sqlite3");
    drop(fixture(&source, false).map_err(|error| error.to_string())?);
    let before = fs::read(&source).map_err(|error| error.to_string())?;
    snapshot_database(&source, &destination)?;
    assert_eq!(
        fs::read(&source).map_err(|error| error.to_string())?,
        before
    );
    let copy = Connection::open(destination).map_err(|error| error.to_string())?;
    assert_eq!(
        values(&copy).map_err(|error| error.to_string())?,
        ["committed"]
    );
    assert_eq!(
        fs::read_dir(root.path())
            .map_err(|error| error.to_string())?
            .count(),
        2
    );
    Ok(())
}

#[test]
fn snapshot_rejects_missing_corrupt_and_existing_destinations() -> Result<(), String> {
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let source = root.path().join("source.sqlite3");
    let destination = root.path().join("snapshot.sqlite3");
    assert!(snapshot_database(&source, &destination).is_err());
    assert!(!source.exists());
    assert!(!destination.exists());
    fs::write(&source, b"not a database").map_err(|error| error.to_string())?;
    assert!(snapshot_database(&source, &destination).is_err());
    assert!(!destination.exists());
    fs::remove_file(&source).map_err(|error| error.to_string())?;
    drop(fixture(&source, false).map_err(|error| error.to_string())?);
    let valid = fs::read(&source).map_err(|error| error.to_string())?;
    let mut corrupt = valid.clone();
    corrupt[100..].fill(0xff);
    fs::write(&source, &corrupt).map_err(|error| error.to_string())?;
    assert!(snapshot_database(&source, &destination).is_err());
    assert!(!destination.exists());
    assert_eq!(
        fs::read(&source).map_err(|error| error.to_string())?,
        corrupt
    );
    assert_eq!(
        fs::read_dir(root.path())
            .map_err(|error| error.to_string())?
            .count(),
        1
    );
    fs::write(&source, valid).map_err(|error| error.to_string())?;
    fs::write(&destination, b"keep me").map_err(|error| error.to_string())?;
    assert!(snapshot_database(&source, &destination).is_err());
    assert_eq!(
        fs::read(&destination).map_err(|error| error.to_string())?,
        b"keep me"
    );
    assert_eq!(
        fs::read_dir(root.path())
            .map_err(|error| error.to_string())?
            .count(),
        2
    );
    assert!(snapshot_database(&source, &source).is_err());
    Ok(())
}

#[test]
fn snapshot_accepts_cleanly_closed_wal_database() -> Result<(), String> {
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let source = root.path().join("source.sqlite3");
    drop(fixture(&source, true).map_err(|error| error.to_string())?);
    assert_eq!(
        fs::read_dir(root.path())
            .map_err(|error| error.to_string())?
            .count(),
        1
    );
    let before = fs::read(&source).map_err(|error| error.to_string())?;
    let destination = root.path().join("snapshot.sqlite3");
    snapshot_database(&source, &destination)?;
    assert_eq!(
        fs::read(&source).map_err(|error| error.to_string())?,
        before
    );
    let copy = Connection::open(destination).map_err(|error| error.to_string())?;
    assert_eq!(
        values(&copy).map_err(|error| error.to_string())?,
        ["committed"]
    );
    Ok(())
}
