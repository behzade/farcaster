use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use serde_json::{Value, json};

use super::{connection::CodexConnection, contract::CodexClientInfo};
use crate::sessions::{SessionSummary, SessionTransfer};

pub(in crate::modules::agents::adapter) fn move_family(
    family: &[SessionSummary],
    destination: &Path,
) -> Result<SessionTransfer, String> {
    let destination = destination
        .canonicalize()
        .map_err(|error| format!("Codex move destination: {error}"))?;
    if !destination.is_dir() {
        return Err("Codex move destination must be a directory".into());
    }
    let directory = destination
        .to_str()
        .ok_or("Codex move destination must be UTF-8")?;
    let program = std::env::var_os("FARCASTER_CODEX_PATH").unwrap_or_else(|| "codex".into());
    move_via_server(Command::new(program), family, directory)
}

fn move_via_server(
    command: Command,
    family: &[SessionSummary],
    directory: &str,
) -> Result<SessionTransfer, String> {
    let root = family.first().ok_or("session family is empty")?;
    let (database, rollouts) = with_server(command, |connection, home| {
        Ok((project_database(home), inspect_family(connection, family)?))
    })?;
    persist_folders(&database, &rollouts, directory)?;
    Ok(SessionTransfer {
        root: root.path.clone(),
        paths: family
            .iter()
            .map(|session| (session.path.clone(), session.path.clone()))
            .collect(),
    })
}

pub(super) fn project_database(home: &Path) -> PathBuf {
    home.join("farcaster-projects.sqlite")
}

pub(super) fn saved_project(database: &Path, id: &str) -> Result<Option<PathBuf>, String> {
    use rusqlite::OptionalExtension;
    if !database.exists() {
        return Ok(None);
    }
    let db =
        rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| error.to_string())?;
    db.query_row(
        "SELECT project FROM session_projects WHERE id = ?1",
        [id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map(|project| project.map(PathBuf::from))
    .map_err(|error| error.to_string())
}

fn persist_folders(
    database: &Path,
    rollouts: &[(String, PathBuf)],
    directory: &str,
) -> Result<(), String> {
    // Never replace or truncate a native rollout: another Codex process can
    // retain an append handle even while this server reports the thread idle.
    for (id, path) in rollouts {
        let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
        let header = BufReader::new(file)
            .lines()
            .next()
            .ok_or("Codex rollout is empty")?
            .map_err(|error| error.to_string())?;
        let header: Value = serde_json::from_str(&header).map_err(|error| error.to_string())?;
        if header["type"] != "session_meta" || header["payload"]["id"].as_str() != Some(id) {
            return Err("Codex rollout identity does not match move target".into());
        }
    }
    let mut db = rusqlite::Connection::open(database).map_err(|error| error.to_string())?;
    db.busy_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
    let tx = db.transaction().map_err(|error| error.to_string())?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_projects (id TEXT PRIMARY KEY, project TEXT NOT NULL)",
    )
    .map_err(|error| error.to_string())?;
    for (id, _) in rollouts {
        tx.execute(
            "INSERT INTO session_projects (id, project) VALUES (?1, ?2)
            ON CONFLICT(id) DO UPDATE SET project = excluded.project",
            rusqlite::params![id, directory],
        )
        .map_err(|error| error.to_string())?;
    }
    tx.commit().map_err(|error| error.to_string())
}

fn with_server<T>(
    mut command: Command,
    operation: impl FnOnce(
        &mut CodexConnection<BufReader<std::process::ChildStdout>, std::process::ChildStdin>,
        &Path,
    ) -> Result<T, String>,
) -> Result<T, String> {
    let mut child = command
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("start Codex move server: {error}"))?;
    let stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let (finished, wait) = mpsc::channel();
    // A missing notification must not leave the supervisor waiting forever.
    let watchdog = thread::spawn(move || {
        let expired = wait.recv_timeout(Duration::from_secs(30)).is_err();
        if !expired {
            // EOF lets Codex flush its rollout and catalog before exiting.
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while std::time::Instant::now() < deadline {
                if let Ok(Some(status)) = child.try_wait() {
                    return !status.success();
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        true
    });
    let mut connection = CodexConnection::new(BufReader::new(stdout), stdin);
    let result = connection
        .initialize_experimental(CodexClientInfo {
            name: "farcaster-move".into(),
            title: Some("Farcaster".into()),
            version: env!("CARGO_PKG_VERSION").into(),
        })
        .and_then(|initialized| operation(&mut connection, Path::new(&initialized.codex_home)));
    drop(connection);
    let _ = finished.send(());
    if watchdog.join().unwrap_or(true) {
        return Err("Codex move timed out; folder changes may be incomplete. Refresh sessions before retrying.".into());
    }
    result
}

fn request<R: BufRead, W: Write>(
    connection: &mut CodexConnection<R, W>,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let id = connection.send_request(method, params)?;
    connection
        .wait_response(&id)
        .map_err(|error| format!("{method}: {error}"))
}

fn inspect_family<R: BufRead, W: Write>(
    connection: &mut CodexConnection<R, W>,
    family: &[SessionSummary],
) -> Result<Vec<(String, PathBuf)>, String> {
    let mut rollouts = Vec::new();
    for session in family {
        // A fresh server must load an existing thread before inspecting its work.
        let response = request(connection, "thread/resume", json!({"threadId":session.id}))?;
        let stored = &response["thread"];
        if stored["id"].as_str() != Some(&session.id) {
            return Err("Codex returned a different thread identity".into());
        }
        if !matches!(
            stored["status"]["type"].as_str(),
            Some("notLoaded" | "idle")
        ) {
            return Err(format!(
                "Codex thread {} must be idle before moving",
                session.id
            ));
        }
        let path = stored["path"]
            .as_str()
            .ok_or("Codex thread has no rollout path")?;
        let queue = request(
            connection,
            "thread/queue/list",
            json!({"threadId":session.id,"limit":1}),
        )?;
        if !queue["data"]
            .as_array()
            .ok_or("Codex returned an invalid queue")?
            .is_empty()
        {
            return Err(format!(
                "Send or remove queued Codex work before moving {}",
                session.id
            ));
        }
        let goal = request(
            connection,
            "thread/goal/get",
            json!({"threadId":session.id}),
        )?;
        if goal["goal"]["status"].as_str() == Some("active") {
            return Err(format!("Pause the Codex goal before moving {}", session.id));
        }
        rollouts.push((session.id.clone(), PathBuf::from(path)));
    }
    Ok(rollouts)
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
