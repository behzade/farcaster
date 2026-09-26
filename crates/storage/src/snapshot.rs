//! Copy SQLite state without invoking application migrations or repairs.

use std::{fs, io::Read as _, path::Path};

use rusqlite::{Connection, OpenFlags};

/// Snapshot committed database contents, including a live WAL, into a new file.
///
/// The caller owns the destination's private directory and must keep the source
/// path stable until this returns. Existing destinations
/// are never replaced. This copies only SQLite contents, not images or session
/// files, and does not migrate or repair application state.
///
/// SQLite opens the source database read-only and takes normal read locks. It
/// may create an empty WAL, create or rebuild shared memory, and update shared
/// memory read marks. These sidecars are SQLite coordination state; the source
/// database's rows and schema are not changed. A source needing database writes
/// for recovery fails rather than being reopened with write access.
pub fn snapshot_database(source: &Path, destination: &Path) -> Result<(), String> {
    let mut file = fs::File::open(source)
        .map_err(|error| format!("open snapshot source {}: {error}", source.display()))?;
    if !file
        .metadata()
        .map_err(|error| format!("inspect snapshot source: {error}"))?
        .is_file()
    {
        return Err("snapshot source must be a regular database file".into());
    }
    let mut header = [0_u8; 100];
    file.read_exact(&mut header)
        .map_err(|error| format!("read snapshot source header: {error}"))?;
    if &header[..16] != b"SQLite format 3\0" {
        return Err("snapshot source is not a SQLite database".into());
    }
    let source =
        std::path::absolute(source).map_err(|error| format!("resolve snapshot source: {error}"))?;
    let mut uri = url::Url::from_file_path(&source)
        .map_err(|()| "snapshot source cannot be represented as a file URL".to_owned())?;
    uri.set_query(Some("mode=ro"));
    let connection = Connection::open_with_flags(
        uri.as_str(),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("open read-only snapshot source: {error}"))?;
    connection
        .busy_timeout(crate::DATABASE_BUSY_TIMEOUT)
        .map_err(|error| format!("configure snapshot lock wait: {error}"))?;

    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let output = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("create database snapshot: {error}"))?;
    let output_path = output
        .path()
        .to_str()
        .ok_or_else(|| "snapshot destination directory must have a UTF-8 path".to_owned())?;
    connection
        .execute("VACUUM main INTO ?1", [output_path])
        .map_err(|error| format!("snapshot SQLite database: {error}"))?;
    output
        .as_file()
        .sync_all()
        .map_err(|error| format!("sync database snapshot: {error}"))?;
    output.persist_noclobber(destination).map_err(|error| {
        format!(
            "publish database snapshot {}: {error}",
            destination.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
