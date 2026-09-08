use super::*;

#[cfg(unix)]
#[test]
fn bounds_output_while_continuing_to_drain() {
    let runner = CommandRunner::new(Duration::from_secs(2), 4, Vec::new());
    let arguments = [
        OsString::from("-c"),
        OsString::from("printf 123456789; printf abcdefghi >&2"),
    ];
    let output = runner
        .run(OsStr::new("sh"), &arguments, Path::new("/"))
        .expect("bounded command should finish");
    assert_eq!(output.stdout, b"1234");
    assert_eq!(output.stderr, b"abcd");
    assert!(output.stdout_truncated && output.stderr_truncated);
}

#[cfg(target_os = "linux")]
#[test]
fn retries_a_temporarily_busy_executable() {
    use std::{fs, os::unix::fs::PermissionsExt as _};

    let temp = tempfile::tempdir().expect("create executable directory");
    let executable = temp.path().join("command");
    fs::write(&executable, "#!/bin/sh\nexit 0\n").expect("write executable");
    let mut permissions = fs::metadata(&executable)
        .expect("read executable metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).expect("make command executable");
    let writer = fs::OpenOptions::new()
        .write(true)
        .open(&executable)
        .expect("hold executable open for writing");
    let release = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        drop(writer);
    });

    let result = CommandRunner::new(Duration::from_secs(1), 1024, Vec::new()).run(
        executable.as_os_str(),
        &[],
        temp.path(),
    );
    release.join().expect("release executable writer");
    assert!(result.expect("retry busy executable").status.success());
}

#[cfg(unix)]
#[test]
fn terminates_the_process_group_after_the_deadline() {
    let marker = std::env::temp_dir().join(format!(
        "pi-repository-timeout-marker-{}",
        std::process::id()
    ));
    let _remove_result = std::fs::remove_file(&marker);
    let runner = CommandRunner::new(Duration::from_millis(20), 1024, Vec::new());
    let arguments = [
        OsString::from("-c"),
        OsString::from("trap 'exit 0' TERM; (trap '' TERM; sleep 0.3; touch \"$1\") & wait"),
        OsString::from("repository-timeout-test"),
        marker.as_os_str().to_os_string(),
    ];
    let error = runner
        .run(OsStr::new("sh"), &arguments, Path::new("/"))
        .expect_err("shell should time out");
    assert!(matches!(error, RepositoryError::CommandTimedOut { .. }));
    std::thread::sleep(Duration::from_millis(400));
    assert!(!marker.exists(), "timed-out descendant was left running");
}
