use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use serde_json::{Value, json};

use super::{
    connection::CodexConnection,
    contract::{CodexClientInfo, CodexInbound},
};
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
    with_server(Command::new(program), |connection| {
        move_with_client(connection, family, directory)
    })
}

fn with_server<T>(
    mut command: Command,
    operation: impl FnOnce(
        &mut CodexConnection<BufReader<std::process::ChildStdout>, std::process::ChildStdin>,
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
        let _ = child.kill();
        let _ = child.wait();
        expired
    });
    let mut connection = CodexConnection::new(BufReader::new(stdout), stdin);
    let result = connection
        .initialize_experimental(CodexClientInfo {
            name: "farcaster-move".into(),
            title: Some("Farcaster".into()),
            version: env!("CARGO_PKG_VERSION").into(),
        })
        .and_then(|_| operation(&mut connection));
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
    connection.wait_response(&id)
}

fn move_with_client<R: BufRead, W: Write>(
    connection: &mut CodexConnection<R, W>,
    family: &[SessionSummary],
    destination: &str,
) -> Result<SessionTransfer, String> {
    let root = family.first().ok_or("session family is empty")?;
    let mut originals = Vec::new();
    // Preflight the entire family before loading or changing any member.
    for session in family {
        let response = request(connection, "thread/read", json!({"threadId":session.id}))?;
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
        let cwd = stored["cwd"]
            .as_str()
            .ok_or("Codex thread has no working directory")?;
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
        if cwd != destination {
            originals.push((session.id.clone(), cwd.to_owned()));
        }
    }
    // Load every native thread before changing any settings.
    for (id, _) in &originals {
        request(connection, "thread/resume", json!({"threadId":id}))?;
    }
    for (index, (id, _)) in originals.iter().enumerate() {
        if let Err(error) = update_cwd(connection, id, destination) {
            let mut failures = Vec::new();
            for (id, cwd) in originals[..=index].iter().rev() {
                if let Err(rollback) = update_cwd(connection, id, cwd) {
                    failures.push(format!("{id}: {rollback}"));
                }
            }
            return Err(if failures.is_empty() {
                format!("Codex move failed; original folders restored: {error}")
            } else {
                format!(
                    "Codex move failed: {error}. Could not confirm restored folders for {}. Refresh sessions before retrying.",
                    failures.join("; ")
                )
            });
        }
    }
    Ok(SessionTransfer {
        root: root.path.clone(),
        paths: family
            .iter()
            .map(|session| (session.path.clone(), session.path.clone()))
            .collect(),
    })
}

fn update_cwd<R: BufRead, W: Write>(
    connection: &mut CodexConnection<R, W>,
    id: &str,
    cwd: &str,
) -> Result<(), String> {
    request(
        connection,
        "thread/settings/update",
        json!({"threadId":id,"cwd":cwd}),
    )?;
    // The RPC response acknowledges admission, not completion or persistence.
    loop {
        match connection.next()? {
            CodexInbound::Notification { method, params }
                if method == "thread/settings/updated"
                    && params["threadId"].as_str() == Some(id)
                    && params["threadSettings"]["cwd"].as_str() == Some(cwd) =>
            {
                return Ok(());
            }
            CodexInbound::Notification { method, params }
                if method == "error" && params["threadId"].as_str() == Some(id) =>
            {
                return Err(format!("Codex settings update failed: {}", params["error"]));
            }
            CodexInbound::ServerRequest { .. } => {
                return Err("Codex requested input while moving a thread".into());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
