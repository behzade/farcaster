use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};

use crate::sessions::{SessionSummary, SessionTransfer};

use super::{
    catalog::with_server,
    client::OpenCodeClient,
    contract::{OpenCodeHttpTransport, OpenCodeLocation},
};

pub(in crate::modules::agents::adapter) fn move_family(
    family: &[SessionSummary],
    destination: &Path,
) -> Result<SessionTransfer, String> {
    let destination = destination
        .canonicalize()
        .map_err(|error| format!("OpenCode move destination: {error}"))?;
    if !destination.is_dir() {
        return Err("OpenCode move destination must be a directory".into());
    }
    let directory = destination
        .to_str()
        .ok_or("OpenCode move destination must be UTF-8")?;
    with_server(|server| {
        move_with_client(
            &mut server.client(),
            family,
            directory,
            Duration::from_secs(10),
        )
    })
}

fn move_with_client<T: OpenCodeHttpTransport>(
    client: &mut OpenCodeClient<T>,
    family: &[SessionSummary],
    destination: &str,
    timeout: Duration,
) -> Result<SessionTransfer, String> {
    let root = family.first().ok_or("session family is empty")?;
    let destination = OpenCodeLocation {
        directory: destination.into(),
        workspace_id: None,
    };
    // Read every member before changing any of them. Use backend locations for rollback.
    let mut originals = family
        .iter()
        .map(|session| {
            let stored = client.get_session(&session.id)?;
            if stored.id != session.id {
                return Err("OpenCode returned a different session identity".into());
            }
            if !client.session_inbox(&session.id)?.is_empty() {
                return Err(format!(
                    "Send or remove queued OpenCode work before moving {}",
                    session.id
                ));
            }
            Ok(stored)
        })
        .collect::<Result<Vec<_>, String>>()?;
    originals.retain(|session| session.location != destination);
    for (index, original) in originals.iter().enumerate() {
        if let Err(error) = move_and_wait(client, &original.id, &destination, timeout) {
            let mut failures = Vec::new();
            // Include the failing request: the server may have accepted it before disconnecting.
            for session in originals[..=index].iter().rev() {
                if let Err(rollback) =
                    move_and_wait(client, &session.id, &session.location, timeout)
                {
                    failures.push(format!("{}: {rollback}", session.id));
                }
            }
            return Err(if failures.is_empty() {
                format!("OpenCode move failed; original folders restored: {error}")
            } else {
                format!(
                    "OpenCode move failed: {error}. Could not confirm restored folders for {}. Refresh sessions before retrying.",
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

fn move_and_wait<T: OpenCodeHttpTransport>(
    client: &mut OpenCodeClient<T>,
    id: &str,
    destination: &OpenCodeLocation,
    timeout: Duration,
) -> Result<(), String> {
    client.move_session(id, destination)?;
    let deadline = Instant::now() + timeout;
    loop {
        let pending_move = client
            .session_inbox(id)?
            .iter()
            .any(|item| item.get("type").and_then(serde_json::Value::as_str) == Some("move"));
        let session = client.get_session(id)?;
        if session.id != id {
            return Err("OpenCode returned a different session identity".into());
        }
        if !pending_move && session.location == *destination {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Timed out waiting for OpenCode session {id} to move"
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
