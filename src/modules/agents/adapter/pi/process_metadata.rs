use std::{ffi::OsStr, path::Path, process::Command};

use super::SessionLaunch;

pub(super) fn apply(
    command: &mut Command,
    project: &Path,
    launch: &SessionLaunch<'_>,
    worker: bool,
    identity: Option<&(String, String)>,
    parent_worker: Option<&str>,
    parent_session: Option<&str>,
) {
    let (mode, resume, fork) = match launch {
        SessionLaunch::Catalog => ("catalog", None, None),
        SessionLaunch::New => ("new", None, None),
        SessionLaunch::Resume(path) => ("resume", Some(path.as_os_str()), None),
        SessionLaunch::Fork(path) => ("fork", None, Some(path.as_os_str())),
    };
    let role = if matches!(launch, SessionLaunch::Catalog) {
        "catalog"
    } else if worker {
        "worker"
    } else {
        "session"
    };
    command
        .env("FARCASTER_PROCESS_APP_PID", std::process::id().to_string())
        .env("FARCASTER_PROCESS_BACKEND", "pi")
        .env("FARCASTER_PROCESS_PROJECT", project)
        .env("FARCASTER_PROCESS_ROLE", role)
        .env("FARCASTER_PROCESS_LAUNCH", mode);
    for (key, value) in [
        (
            "FARCASTER_PROCESS_WORKER_ID",
            identity.map(|(id, _)| OsStr::new(id)),
        ),
        (
            "FARCASTER_PROCESS_WORKER_NAME",
            identity.map(|(_, name)| OsStr::new(name)),
        ),
        (
            "FARCASTER_PROCESS_PARENT_WORKER_ID",
            parent_worker.map(OsStr::new),
        ),
        (
            "FARCASTER_PROCESS_PARENT_SESSION",
            parent_session.map(OsStr::new),
        ),
        ("FARCASTER_PROCESS_RESUME_FILE", resume),
        ("FARCASTER_PROCESS_FORK_SOURCE", fork),
    ] {
        match value {
            Some(value) => {
                command.env(key, value);
            }
            None => {
                command.env_remove(key);
            }
        }
    }
}

#[cfg(test)]
#[path = "process_metadata_tests.rs"]
mod tests;
