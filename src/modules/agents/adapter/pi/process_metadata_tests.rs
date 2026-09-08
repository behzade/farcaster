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
