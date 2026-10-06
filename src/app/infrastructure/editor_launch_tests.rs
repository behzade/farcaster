use super::*;
use std::os::unix::{ffi::OsStringExt as _, fs::PermissionsExt as _};
use std::process::{Command, Output};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn run_launch(command: &str) -> std::io::Result<Output> {
    Command::new("/bin/sh").args(["-c", command]).output()
}

#[test]
fn prepare_launches_from_ghostty_without_login_scripts_or_environment_in_command() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("launch.sh");
    let marker = directory.path().join("profile-ran");
    std::fs::write(
        directory.path().join(".profile"),
        format!("touch '{}'\n", marker.display()),
    )?;
    crate::agents::set_test_project_environment(
        directory.path(),
        vec![(
            "FARCASTER_LAUNCH_TEST_SECRET".into(),
            "private value".into(),
        )],
    );
    let command = prepare(
        &path,
        Path::new("/usr/bin/env"),
        &["-0".into()],
        directory.path(),
    )?;
    assert!(!command.contains("private value"));
    let output = Command::new("bash")
        .args(["--noprofile", "--norc", "-c", &format!("exec -l {command}")])
        .env("HOME", directory.path())
        .output()?;
    assert!(output.status.success(), "{:?}", output.stderr);
    assert!(
        output
            .stdout
            .split(|byte| *byte == 0)
            .any(|entry| entry == b"FARCASTER_LAUNCH_TEST_SECRET=private value")
    );
    assert!(!marker.exists());
    assert!(!path.exists());
    Ok(())
}

#[test]
fn launch_preserves_project_environment_with_ghostty_terminal_settings() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("it's $HOME `not a command`.sh");
    let environment = vec![
        ("PATH".into(), "/captured/project/bin".into()),
        (
            "FARCASTER_CAPTURED_PROJECT_PATH".into(),
            "/captured/project/bin".into(),
        ),
        ("EMPTY".into(), "".into()),
        (
            "QUOTED".into(),
            "it's $HOME\n`not a command` = value".into(),
        ),
        ("BYTES".into(), OsString::from_vec(vec![0xff, b'x'])),
        ("TERM".into(), "dumb".into()),
        ("COLORTERM".into(), "".into()),
        ("TERM_PROGRAM".into(), "another-terminal".into()),
    ];
    let command = write_launch(
        &path,
        Path::new("/usr/bin/env"),
        &["-0".into()],
        directory.path(),
        &environment,
    )?;
    assert_eq!(
        std::fs::metadata(&path)?.permissions().mode() & 0o777,
        0o600
    );
    let output = Command::new("/bin/sh")
        .args(["-c", &command])
        .env("FARCASTER_UNCAPTURED", "must not leak")
        .output()?;
    assert!(
        !path.exists(),
        "consume the private environment snapshot before launch"
    );
    assert!(output.status.success());
    let mut actual: Vec<Vec<u8>> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(Vec::from)
        .collect();
    let mut expected: Vec<Vec<u8>> = environment
        .iter()
        .filter(|(key, _)| !matches!(key.to_str(), Some("TERM" | "COLORTERM" | "TERM_PROGRAM")))
        .map(|(key, value)| {
            let mut entry = key.as_bytes().to_vec();
            entry.push(b'=');
            entry.extend_from_slice(value.as_bytes());
            entry
        })
        .collect();
    expected.extend([
        b"TERM=xterm-256color".to_vec(),
        b"COLORTERM=truecolor".to_vec(),
        b"TERM_PROGRAM=gpui-ghostty".to_vec(),
    ]);
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn launch_uses_captured_path_and_preserves_argument_bytes() -> TestResult {
    let directory = tempfile::tempdir()?;
    std::os::unix::fs::symlink("/bin/sh", directory.path().join("editor"))?;
    let path = directory.path().join("launch.sh");
    let argument = OsString::from_vec(b"a 'quoted' $argument\n\xff`".to_vec());
    let command = write_launch(
        &path,
        Path::new("editor"),
        &[
            "-c".into(),
            "test ! -e \"$1\" && printf '%s' \"$2\"".into(),
            "editor".into(),
            path.as_os_str().into(),
            argument.clone(),
        ],
        directory.path(),
        &[("PATH".into(), directory.path().as_os_str().into())],
    )?;
    let output = run_launch(&command)?;
    assert!(output.status.success());
    assert_eq!(output.stdout, argument.as_bytes());
    Ok(())
}

#[test]
fn custom_editor_launch_preserves_options_and_starts_in_project() -> TestResult {
    let directory = tempfile::tempdir()?;
    let project = directory.path().canonicalize()?;
    let program = project.join("my editor");
    std::fs::write(&program, "#!/bin/sh\nprintf '%s\\0' \"$PWD\" \"$@\"\n")?;
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))?;
    let command = farcaster_editors::EditorCommand::parse(&format!(
        "'{}' -p 'a \"quote\"' '$HOME'",
        program.display()
    ))?;
    let mut arguments = command.arguments.clone();
    arguments.extend(command.project_arguments(&project));
    let path = project.join("launch.sh");
    let command = write_launch(&path, &command.program, &arguments, &project, &[])?;
    let output = run_launch(&command)?;
    assert!(output.status.success());
    let values: Vec<_> = output.stdout.split(|byte| *byte == 0).collect();
    assert_eq!(
        values,
        [
            project.as_os_str().as_bytes(),
            b"-p",
            b"a \"quote\"",
            b"$HOME",
            b""
        ]
    );
    Ok(())
}

#[test]
fn null_argument_does_not_write_a_script() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("launch.sh");
    assert!(
        write_launch(
            &path,
            Path::new("/bin/sh"),
            &["a\0b".into()],
            directory.path(),
            &[],
        )
        .is_err()
    );
    assert!(!path.exists());
    Ok(())
}

#[test]
fn failed_editor_launch_consumes_private_script() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("launch.sh");
    let command = write_launch(
        &path,
        &directory.path().join("missing-editor"),
        &[],
        directory.path(),
        &[],
    )?;
    let output = run_launch(&command)?;
    assert!(!output.status.success());
    assert!(!path.exists());
    Ok(())
}

#[test]
fn launch_uses_host_path_for_cleanup_before_restoring_project_environment() -> TestResult {
    let directory = tempfile::tempdir()?;
    let host_bin = directory.path().join("host 'tools");
    let project_bin = directory.path().join("project-tools");
    std::fs::create_dir(&host_bin)?;
    std::fs::create_dir(&project_bin)?;
    let cleanup = Command::new("/bin/sh")
        .args(["-c", "command -v rm"])
        .output()?;
    assert!(cleanup.status.success());
    let cleanup = OsString::from_vec(cleanup.stdout.strip_suffix(b"\n").unwrap().to_vec());
    std::os::unix::fs::symlink(Path::new(&cleanup), host_bin.join("rm"))?;
    // Project tools are not available until after the private script is removed.
    let project_rm = project_bin.join("rm");
    std::fs::write(&project_rm, "#!/bin/sh\nexit 99\n")?;
    std::fs::set_permissions(&project_rm, std::fs::Permissions::from_mode(0o700))?;
    let path = directory.path().join("launch.sh");
    let command = write_launch(
        &path,
        Path::new("/usr/bin/env"),
        &["-0".into()],
        directory.path(),
        &[("PATH".into(), project_bin.as_os_str().into())],
    )?;
    let output = Command::new("/bin/sh")
        .args(["-c", &command])
        .env_clear()
        .env("PATH", &host_bin)
        .env("FARCASTER_UNCAPTURED", "must not leak")
        .output()?;
    assert!(output.status.success(), "{:?}", output.stderr);
    assert!(!path.exists());
    let actual: Vec<_> = output.stdout.split(|byte| *byte == 0).collect();
    let mut expected_path = b"PATH=".to_vec();
    expected_path.extend_from_slice(project_bin.as_os_str().as_bytes());
    assert!(actual.contains(&expected_path.as_slice()));
    assert!(
        !actual
            .iter()
            .any(|entry| entry.starts_with(b"FARCASTER_UNCAPTURED="))
    );
    Ok(())
}
