//! Launch-time diagnostics inherited by Pi and its actions on macOS and Linux.
//! These are labels, not credentials or authoritative live session state.
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
    // Set or remove each optional field once, overriding stale labels from
    // the captured login environment or an agent that launched the desktop.
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
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn environment(command: &Command) -> BTreeMap<&str, Option<&str>> {
        command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_str().unwrap(),
                    value.map(|value| value.to_str().unwrap()),
                )
            })
            .collect()
    }

    #[test]
    fn worker_metadata_preserves_labels_without_changing_arguments() {
        let mut command = Command::new("pi");
        command.args(["--mode", "rpc"]);
        let identity = ("worker-42".into(), "review spaces / 日本語".into());
        apply(
            &mut command,
            Path::new("/project with spaces"),
            &SessionLaunch::New,
            true,
            Some(&identity),
            Some("parent-7"),
            Some("native-parent"),
        );
        let env = environment(&command);
        assert_eq!(env["FARCASTER_PROCESS_ROLE"], Some("worker"));
        assert_eq!(
            env["FARCASTER_PROCESS_WORKER_NAME"],
            Some(identity.1.as_str())
        );
        assert_eq!(env["FARCASTER_PROCESS_PARENT_WORKER_ID"], Some("parent-7"));
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["--mode", "rpc"]);
    }

    #[test]
    fn launch_modes_clear_stale_parent_and_session_metadata() {
        for (launch, role, field) in [
            (SessionLaunch::Catalog, "catalog", None),
            (SessionLaunch::New, "session", None),
            (
                SessionLaunch::Resume(Path::new("/resume")),
                "session",
                Some("FARCASTER_PROCESS_RESUME_FILE"),
            ),
            (
                SessionLaunch::Fork(Path::new("/source")),
                "session",
                Some("FARCASTER_PROCESS_FORK_SOURCE"),
            ),
        ] {
            let mut command = Command::new("pi");
            command.env("FARCASTER_PROCESS_PARENT_WORKER_ID", "stale");
            command.env("FARCASTER_PROCESS_RESUME_FILE", "stale");
            apply(
                &mut command,
                Path::new("/project"),
                &launch,
                false,
                None,
                None,
                None,
            );
            let env = environment(&command);
            assert_eq!(env["FARCASTER_PROCESS_ROLE"], Some(role));
            assert_eq!(env["FARCASTER_PROCESS_PARENT_WORKER_ID"], None);
            for key in [
                "FARCASTER_PROCESS_RESUME_FILE",
                "FARCASTER_PROCESS_FORK_SOURCE",
            ] {
                assert_eq!(env[key].is_some(), field == Some(key));
            }
        }
    }
}
